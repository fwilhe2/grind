<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# The tape — a scratch calculator that becomes a spreadsheet

**Status: an experiment, built to be tried and removed if it does not earn its place.** The
named expression list, the tape, extracting a constant (on a tape and on a sheet cell),
promotion, `grind calc`, the terminal's tape pane and the spreadsheet window's tape dialog are
built. §8 lists what is not.

## 1. What it is

A list of lines, each an OpenFormula expression with no leading `=`, evaluated as you type:

```
#1   vat = 19%        0.19
#2   net = 1200       1200
#3   net*(1+vat)      1428
#4   #3/12            119
```

Every line has a handle (`#n`). A line can name itself (`vat = …`), and any line can read
another through its handle or its name. **Promote** turns the tape into a spreadsheet: one row
per line, names as named ranges, handles as cell references. **Extract constant** moves a number
out of a line into a named input line. On a sheet cell, it moves the number into a named input
cell.

## 2. How this fits `doc/formula-workbench.md` decision 1

That decision rules out `x = 5` lines, because a scratch language would be a language no cell
can hold (R1). The tape reads it this way:

> **A tape is a sheet that has not been laid out yet.** It adds exactly two spellings, and each
> one is a name for something a cell already holds. `name =` is a named range over the line's
> cell. `#n` is a reference to row *n*. There are no statements, no reassignment, no loops and
> no scopes. Every tape without a cycle promotes, and the promoted sheet computes the same values
> because the tape is evaluated *as* that sheet (§3).

That reading is the experiment's premise. If it does not hold up in use, the tape goes and the
decision stands as written.

## 3. The model

Everything lives in `grind-sheet`, because the engine is there and R8 keeps it out of
`grind-core`.

| Item | Where | Holds |
|---|---|---|
| `ExprList` | `formula/list.rs` | the reusable named expression list: entries, the dependency graph, cycles, incremental evaluation, rewrites |
| `Tape` | `calc.rs` | line syntax, `#n`, locale numbers, display, undo, promotion |
| `App::extract_constant` | `app/names.rs` | the sheet-cell extract |

**The list is a column of cells.** Entry *i* is cell `B(i+1)` of a private one-sheet document.
A named entry is a document name over that cell (`[$Sheet1.$B$3]`). The engine evaluates it
through its own memo and error model, so there is no second evaluator. Promotion writes the same
document with labels added. Inside the list, a reference to another entry is `[.Bn]` or a name.
`#n` exists only in the tape's spelling.

**The graph.** For each entry, the list walks its parsed formula to find which entries it reads:
a reference into column B, a range over it, or a name. A strongly connected components pass
(Tarjan) finds the cycles. Every member of a cycle is **`#NUM!`**, which is what the engine
already returns for a circular reference. Entries that read a cycle get the error through
ordinary propagation. The cycle itself is reported by handle. LibreOffice shows `Err:522` for
the same thing, and Part 4 names no error for it.

**Incremental.** An edit to entry *i* recomputes *i* and everything downstream of it. The
engine is seeded with every other entry's cached value (`Engine::with_overlay`). The edit returns
the entries whose value changed, which is what the ODT fields will need. A seeded random test
holds incremental evaluation equal to evaluating from nothing. Inserting or removing an entry
recomputes everything. Sheets keep recomputing everything: `doc/not-doing.md`'s row is unchanged.

**Insert and remove** move every reference past the point. A reference to a removed entry is
`#REF!`, and a range whose end is removed shrinks. A single reference past the last entry is
`#REF!`, not an empty cell.

**Names** follow `App::set_name`'s rules and are defined once. Renaming a line carries its
uses with it. Dropping a line's name turns its uses into references.

**Undo** is a stack of whole lists inside `Tape`: a tape is small, and a snapshot cannot
disagree with the edit it reverses.

`ExprList` is crate-private until a second caller needs it. The ODT task (variable fields) makes
it public, and decides whether `grind-text` reaches the engine through `grind-sheet` or through
a crate of its own.

## 4. Line syntax

```
line := '=' expr          a comparison or any expression, never a definition
      | name '=' expr     a named line
      | expr
```

- A line that starts with a name followed by `=` is a definition. To compare, start the line
  with `=` (`=vat=0.19`) or wrap it in parentheses.
- `#n` outside a string is line *n*.
- A cell address (`B2`, `[.B2]`) is refused with *a tape has no cells: name a line or write #n*.
  Without that, it would read the hidden column.
- **Numbers are read in the tape's locale**, with the CSV import's rule (`csv::number`).
  OpenFormula separates arguments with `;`, so `,` is never ambiguous. In de-DE,
  `1.234,5 * 2` is 2469 and `1.5` is refused. Values and lines are shown in the same locale.
- Values show fifteen significant digits, as a spreadsheet does: `0.1 + 0.2` is `0.3`.
- A circle is said in words by the tape itself (`Tape::circles`): `#3 reads itself`, or `#1, #2
  read each other in a circle`, so every client says it the same way.

## 5. Extract constant

**On a tape:** `Tape::extract(line, number, nth, name)`. The number is found by value among the
line's literals, and `nth` (counting from 1) picks one when there are several. A line
`name = <number>` is inserted **directly before** the line, and the line reads `name`. A `%`
after the number goes with it (`5%` becomes `rate = 5%`). It is one undo step.

**On a sheet cell:** `App::extract_constant(sheet, pos, number, nth, target, name)` is the first
slice of the workbench's extraction (`doc/formula-workbench.md` §5). As one `Action::Batch`, it:

- writes the number (divided by 100 after a `%`) into the empty cell `target`,
- names that cell, and
- has the formula read the name.

It is refused when:

- the target is not empty,
- the name is taken or not a name,
- the number is not in the formula, or
- the **dynamic check** finds that the answer changed. For example, `ISREF(3)` is FALSE, but
  `ISREF(name)` is TRUE.

## 6. Promotion

| Tape | Promoted sheet (`Sheet1`) |
|---|---|
| line *n* | row *n*; no header, so `#n` is row *n* |
| column A | the line's name, or `#n` |
| a line that is only a number (signed, or a `%`) | its value in B |
| any other line | its formula in B, with its value cached |
| `#k` | the name of line *k* if it has one, else `[.Bk]` |
| `name = …` | a named range over `$Sheet1.$B$n` |
| the tape's locale | the document's locale |

There is no tape metadata in the file. The format comes from the extension (`App::save_file`):
`.ods`, `.grind`, and anything else flat. **A tape with a cycle is not promoted**, because
LibreOffice and grind name that error differently. **A tape has no file format of its own**:
promotion is how one is kept, and a text file of lines piped into `grind calc` is how one is
replayed.

## 7. Where to reach it

```sh
grind calc '1200 * 1.19'                                  # 1428
grind calc 'vat = 0.19' 'net = 1200' 'net*(1+vat)'        # one row per line
grind calc < lines.txt                                     # stdin, when it is not a terminal
grind calc                                                 # interactive, at a terminal
grind calc … --locale de-DE
grind calc … --extract '#3' 0.19 vat [--nth 2]             # prints the tape afterwards
grind calc … --save tape.fods [--force]                    # promote
grind --format json calc …
grind sheet extract book.fods B1 0.19 --to C1 --name vat [--nth 2]
```

The interactive tape takes a plain line to append, and `:list`, `:edit N <line>`,
`:insert N <line>`, `:del N`, `:extract N <number> <name> [K]`, `:undo`, `:redo`,
`:save <file> [!]`, `:help` and `:q`. A line that does not read is status 2. A refused save or
extract is status 1. `doc/cli-parity-calc.md` maps every `Tape` method to its spelling.

**The terminal shell** has a tape pane (`:tape`, or `grind-tui --tape`); `doc/tui-shell.md` has
its keys. **The GNOME spreadsheet window** has a tape dialog (*Scratch Calculator*, or
`grind-sheet-gtk --tape`) and *Extract a Number into a Named Cell…*; `doc/sheet-shell.md` says
how. The other graphical shells have neither (`doc/feature-matrix.md`).

## 8. Not built

- **A formula filled down a column**, on a sheet: extracting from one copy rewrites only that
  copy. The workbench says to refuse with the count; that check is not here yet.
- **Array formulas and covered cells**, on a sheet: not refused by name.
- **Reading a promoted sheet back as a tape.**
- **A percentage format** on a promoted `%` input: it holds 0.19 and shows `0.19`.
- **Tests not run here**: `promoted_tapes_are_valid_odf` skips without `jing` on `PATH`.
  `scripts/claude-vm.sh` has it.
