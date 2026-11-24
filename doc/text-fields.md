<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# Live calculations in text — variable, user and expression fields

**Status: a design, for review. Nothing here is built.** It extends the named expression list
from `doc/calc.md` §3 to ODF's own text fields, so a letter or a report can say *3 × 100 = 300*
and keep it true. It does not embed spreadsheets.

## 1. The namespace question, answered by the oracle

Writer writes `text:formula="ooow:…"`, which is its own calculator syntax, not OpenFormula. The
question was whether Writer evaluates `of:` too. It does not, and it does something worse than
ignoring it.

All facts below are **MEASURED** (2026-11-24, LibreOffice 26.8.3.1). Each probe was a
hand-written `.fodt` with stale cached values. It was laid out by `soffice --convert-to pdf`,
read with `pdftotext`, and saved by `soffice --convert-to fodt` and through UNO after
`TextFields.refresh()`. The probes go into `doc/odt-format.md` §5 as numbered facts in the first
commit, before any code.

| # | Fact | Evidence |
|---|---|---|
| F1 | An `ooow:` formula is evaluated, both by layout and by a field refresh: `ooow:2+3` gives 5, `ooow:a*10` gives 50, and `sum 2\|3`, `2^10`, `2 pow 10`, `2 mul 3`, `sqrt 16`, `3 L 4` and `(1+2)*3` all compute | probe 1, 2 |
| F2 | **An `of:` formula is dropped on load.** `of:=2+3`, `of:2+3` and an unprefixed `2+3` all lose their `text:formula` attribute. Writer shows the cached display text. A `variable-set` keeps its `office:value` as a constant, and an `expression`'s `office:value` is reset to 0. A refresh shows 0, and a save writes the field with no formula | probe 1, 7 |
| F3 | **`ooow:` numbers are read in the document's language.** In a `de-DE` document, `ooow:1.5*2` is *Expression is faulty* and `ooow:1.000*2` is 2000. In `fr-FR`, `1.5*2` is not evaluated at all. With no language set, Writer uses its own locale (here en-US). The corpus shows the same thing: `tdf125154.odt` caches `ooow:5.000*10%` as 5000 | probe 3, 4 |
| F4 | An exponent spelling has no separator, and it reads the same everywhere: `ooow:15E-1*2` is 3 in de, en, fr and with no language | probe 4 |
| F5 | **Variables are assigned in document order.** A `variable-get`, or a name in a formula, reads the nearest `variable-set` of that name *before* it. With no set before it, it reads 0. `n = n+1` reads the previous `n`, so it counts. The corpus does this: `variable-field-table-row-split-header.fodt` sets `CARRYOVER_VALUE` four times | probe 2 |
| F6 | **A user field's formula is not kept.** Writer never writes `text:formula` on a `text:user-field-decl`, even for a field it created with the formula `price*1.19`. On load it ignores one written by anyone else. A user field is a value | probe 1, 2, author |
| F7 | A user-field cycle (`cyA = cyB+1`, `cyB = cyA+1`) shows 0 with no error | author |
| F8 | A failed evaluation (`1/0`, or `round(2.567\|2)`, which is not Writer's syntax) shows `** Expression is faulty **` with `office:value="1.7976931348623157E+308"`. An unknown name (`nosuch+1`) is not updated at all: the old display text stays | probe 2 |
| F9 | Writer drops `text:formula` from a `variable-set` whose formula is a plain number equal to its value (`ooow:1` becomes no formula), and it adds `style:data-style-name="N0"` to every numeric field | probe 2, author |
| F10 | Layout (PDF export) evaluates fields. `--convert-to txt` and `--convert-to fodt` do not, unless a refresh runs first | probe 5, 6 |

**What this means for each option.**

- **Write `of:`.** This is worse than "LibreOffice shows cached values". Writer shows them, but
  its first save loses every formula (F2). A grind document opened and saved in Writer comes
  back with its calculations frozen. That breaks architecture rule 6 (every feature survives a
  LibreOffice round trip) for every field. Rejected as the default.
- **Write `ooow:` only.** Writer keeps and evaluates these. But Writer's syntax cannot spell an
  external reference, a string or most functions, and its numbers depend on the locale (F3).
  On its own it cannot carry the feature.
- **Both, with a defined default.** This is the decision (§3).

## 2. Coverage

| Element | rng | Read | Evaluated | Written by grind |
|---|---|---|---|---|
| `text:variable-decls` / `text:variable-decl` | 17023, 18491 | name, `office:value-type` | — | a new decl when a new name is first set, in the prelude's schema order (variable-decls, sequence-decls, user-field-decls) |
| `text:variable-set` | 8611 | name, `text:formula`, value and type, `text:display`, display text | yes | yes |
| `text:variable-get` | 8621 | name, `text:display`, display text (it has no value attributes) | yes | yes |
| `text:expression` | 8664 | `text:formula`, optional value and type, `text:display`, display text | yes | yes |
| `text:user-field-decls` / `text:user-field-decl` | 17037, 18390 | name, optional `text:formula`, value and type | yes (a formula from another producer is honoured) | value only, as Writer does (F6) |
| `text:user-field-get` | 8639 | name, `text:display` (value, formula or none), display text | yes | yes |

The value types are `float`, `percentage` and `currency` (with `office:value`), `string`, `date`,
`time` and `boolean`.

**The first cut evaluates numeric fields only:** float, percentage and currency. A field of
another type is read, shown with its cached text, listed, and written back as its own bytes.
Recalc leaves it alone. Writer's string variables take the formula text literally
(`tdf103025.odt`: `ooow:2014-03` is the string `2014-03`). That needs its own measurements.

**Preserving what is not owned.** Each field stays spliced, like today's `Run::Kept`. Its
original start tag is carried, and a recalc rewrites only the attributes it owns
(`office:value`, `office:value-type`, and later `office:string-value`, `-date-value`,
`-time-value`, `-boolean-value`) plus the display text. `style:data-style-name`, `text:display`
and any foreign attribute (`loext:…`) are carried byte for byte. An unedited, unrecalculated
document is written byte-identical (R6, rule 7).

**Not in scope:** `text:variable-input` and `text:user-field-input`, which ask the reader for a
value; `text:sequence` (numbering); `text:table-formula`; Writer's `<A1>` table-cell references;
and the conditions on `text:hidden-paragraph`, `text:hidden-text` and `text:conditional-text`.
Those conditions read variables too. They stay kept bytes, and after a recalc they can be
stale. `grind text fields` says so when a document has one.

**Display text** is the value formatted with the field's `style:data-style-name`, read with the
spreadsheet's number-format reader. A style it cannot read falls back to General: fifteen
significant digits in the document's language. LibreOffice's default `N0` shows `0.25` in en
and `0,25` in de (probe 4). `text:display="none"` shows nothing. `text:display="formula"` shows
the formula. **UNVERIFIED**: whether that shows the prefix. It is not implemented until it is
measured.

## 3. Formula syntax: read both, write `ooow:` when it is exact

- **Stored verbatim.** A field's `text:formula` is never rewritten unless that field is edited.
  That holds for `ooow:`, `of:` and anything else.
- **Read both.** `of:=…` goes through the engine's parser. `ooow:…` goes through a translator
  into the same AST. The translator covers a closed table, and every row of it is a MEASURED
  fact first: `+ - * / ^`, unary minus, `%`, parentheses, `pow`, `mul`, `div`, `sqrt`, `abs`,
  `sum`/`mean`/`min`/`max` with `|`, and the comparison and logical keywords. Each row is
  measured for both evaluation and round trip before it is implemented. Numbers are read in the
  document's default language (F3). A formula in a run whose language differs from the default
  is **UNVERIFIED**: recalc refuses it and leaves its cached value, saying why. An `ooow:`
  formula outside the table is not evaluated. It keeps its cached value and is listed as
  *Writer's syntax, not read: uses `<A1>`*.
- **Write `ooow:` by default.** People type OpenFormula, as in a cell (`=qty*1.19`). When the
  formula's AST is inside the table, grind writes the `ooow:` spelling. Non-integers are written
  as integer-mantissa exponents (`15E-1`), which read the same in every locale (F4). The
  translation is checked by translating it back: the result must be the same AST, or grind
  writes `of:`.
- **Write `of:=` when it cannot be `ooow:`.** That happens for an external reference, a string,
  or a function outside the table. grind writes `of:=…` and declares `xmlns:of` (as in
  b9e4c80). The verb, `grind lint` and the shell all say: *LibreOffice will show this
  field's value but drop its formula when it saves*. There is no option to pick the syntax
  (`doc/simplicity.md`). The rule decides.
- **Names.** A Writer name that is not an OpenFormula identifier (`Signer.Person.Name`, or
  names with spaces) is spelled `$$'Signer.Person.Name'` in OpenFormula (Part 4 §5.11). The
  lexer already reads it.

## 4. The mapping to `ExprList`

`ExprList` (`sheet/src/formula/list.rs`) becomes `pub`. `grind-text` gains a dependency on
`grind-sheet`. That is the question `doc/calc.md` §3 left open. It costs no binary anything:
every shell that opens text already links `grind-sheet` through `grind-open`. R8 is about
`grind-core` and is untouched. A separate formula crate would split the engine from the
`Document` it evaluates over. That is a large refactor with no second caller.

Document order (F5) makes variables a chain of definitions, not a set of names. The list is
built from the document in reading order: the prelude, then the body, including headers,
footers, tables and notes in the order layout meets them. One entry per field:

| Field | Entry | Its formula, as the list sees it |
|---|---|---|
| `user-field-decl` *u* | named `u` | its value, or its formula from another producer |
| `variable-set` of *x* (the *k*th) | unnamed | the field's formula, with each name *y* rewritten to `[.Bn]` of the nearest earlier set of *y*, or to `u` for a user field, or to `0` when nothing earlier sets it (F5) |
| `variable-get` of *x* | unnamed | `[.Bn]` of the nearest earlier set of *x*, or `0` |
| `expression` | unnamed | its formula, rewritten the same way |
| `user-field-get` *u* | unnamed | `u` |

- **Every reference to a variable points backwards**, so variables cannot form a cycle. `n =
  n+1` reads the previous `n`. Only user fields can be circular. Every member of a circle is
  `#NUM!`, as on the tape and in a sheet, and the circle is named in words
  (`Tape::circles`'s phrasing moves to `ExprList`). Writer shows 0 there (F7). That divergence
  is deliberate and documented.
- **A name that exists as both a variable and a user field** is **UNVERIFIED** in Writer.
  Recalc refuses the document with the name.
- **Incremental.** Editing a field changes one entry. `ExprList::set` returns the entries whose
  value changed, and those fields get new display text through `App::mutate`. That is what
  makes it reactive in a shell. Inserting or removing a field, or renaming a set, changes the
  chain, so the list is rebuilt. It is small, so that is fine.
- **Errors on the page.** An error field shows its OpenFormula name (`#DIV/0!`, `#NAME?`,
  `#NUM!`). ODF has no error value type for fields, so the file gets the declared value type
  with `office:value="0"`, and the error name as display text. That is the spreadsheet's
  error-cell convention without its `calcext:` marker, which the schema does not allow here.
  Writer would show `** Expression is faulty **` (F8). grind's text names the error. Reading
  such a field back trusts the display text only until the next recalc.
- **Undo** is the text core's own: an edit to a field is an `Action`, and `Document::apply`
  returns its inverse with the old cached values.

## 5. External references and the security model

A field can read a spreadsheet next to it in OpenFormula's own external syntax (Part 4 §5.8,
§5.11):

```
=['budget.ods'#$Sheet1.B4] * 1.19      a cell
='budget.ods'#Total                    a named range or expression
=SUM(['budget.ods'#$Sheet1.B2:.B9])    a range, inside a function
```

These are always written as `of:=` (§3). Writer cannot spell them, so after Writer saves the
file they are frozen values (F2).

**Nothing is read on open, and nothing is evaluated on open.** Opening shows each field's cached
value. Editing a field in a shell recomputes only inside the document. A field downstream that
reads an external reference uses that reference's cached value. **External files are read only
by an explicit recalc**: `grind text recalc`, or the shells' *Recalculate* command.

| Rule | Why |
|---|---|
| The core never touches the filesystem. `App::recalc` takes a resolver, `Fn(&str) -> Result<Vec<u8>>`, and the CLI, TUI and server each supply one | rule 5; the web page has none, so its external references keep their cached values and it says so |
| The core accepts only a **relative IRI** or a `file:` IRI before asking the resolver. Every other scheme (`http:`, `https:`, `smb:`, `ftp:`, …) is `#REF!` with *grind does not read other computers* | R12, checked in two places |
| The filesystem resolver resolves against the text document's directory, canonicalises the path (following symlinks), and refuses anything that ends up outside that directory and below it, including `..` and absolute paths elsewhere | "a sibling `.ods`"; a document cannot point at `~/.ssh` |
| The bytes must sniff as a spreadsheet (`grind_core::kind`), never trusting the extension, and are read by the tolerant ODS reader. No import filter, no `.xlsx` in the first cut | one reader, already fuzzed by the corpus |
| The sheet's **cached values** are what the field reads. The sheet is not recalculated, and its own external references are not followed | no transitive reads; the numbers are the ones the sheet's author saw saved |
| Each source is read once per recalc, with a cap on the number of distinct sources (16) and the tolerant reader's own size limits | one recalc cannot fan out across a directory |
| Recalc prints every file it read. A missing or refused source gives its fields `#REF!`, prints the reason, and exits 1, keeping the other results | the user sees exactly what was touched |

The engine gains `Engine::with_sources(map from IRI to &Document)`. `Reference::source`, which is
"never evaluated" today, is looked up there. A reference with no map behaves as it does now, so
sheets are unchanged.

## 6. The surface

**Addressing.** `@name` addresses the first `variable-set` of a name, or else its user field.
`@name[2]` addresses the second set. `@'Signer Name'` quotes a name with spaces or other
punctuation. The parser in `text/src/loc.rs` reads it next to `p12`, `#bookmark` and `§2.1`, and
`+offset` composes as it does for the others. An anonymous expression is addressed by its place
(`p12+40` lands on it) or by its number in `grind text fields`.

**CLI** (rows in `doc/cli-parity-text.md`, lines in `examples/sample-text.sh`):

```sh
grind text fields report.fodt                       # #, kind, name, formula, value, where, notes
grind text field report.fodt p3+12 set qty '=3'     # insert a variable-set at a caret
grind text field report.fodt p4+0 show qty          # insert a variable-get
grind text field report.fodt p5+8 calc '=qty*price' # insert an expression
grind text field report.fodt @qty '=4'              # edit a field's formula (or `@qty[2]`, `p5+8`)
grind text input report.fodt price 100              # define or change a user field (a value)
grind text field report.fodt p6+0 show price        # a user-field-get when `price` is a user field
grind text recalc report.fodt                       # recompute everything, external files included
grind --format json text fields report.fodt
```

The typing rule is the spreadsheet's: a leading `=` is a formula, anything else is a value in
the document's language. Erasing a field is the existing `grind text erase` over its caret. A
verb that writes `of:=` says why (§3). Each verb is one undo step.

**TUI.** A field shows its value inline, as a kept field does now. When the caret is on a
field, the status line shows the formula in the plain-language spelling
(`formula::friendly::explain_inline`): `qty × price  =  300`, or *Writer's syntax, not read*,
or the error and its cause. `=` on a field opens the formula on the command line to edit.
`:field set|show|calc …`, `:input`, `:fields` (a pick list that jumps to a field) and
`:recalc` mirror the CLI. `:recalc` is the only thing that reads another file.

**Other shells.** They already show every field's cached value, and they keep it correct
after an edit beside it. Formula on focus, editing and Recalculate in GTK, web, Win32 and
Mac go in a follow-up, as `○` in `doc/feature-matrix.md`. The core API is the same one the TUI
uses.

**Lint.** `grind lint` warns about an `of:` field (*LibreOffice will drop this formula*), an
`ooow:` field outside the table (*not evaluated by grind*), and a condition that reads a
variable (*not updated by recalc*).

## 7. Tests

| Test | Holds |
|---|---|
| `text/tests/fields.rs`: fixtures | Vendored under `text/tests/data/fields/` (CC0 in `REUSE.toml`). Writer-authored by a committed UNO script (`scripts/fields-fixtures.py`, reproducible against the pinned image), plus the probes above. Covers counters, re-sets, get before set, user fields, a user-field cycle, `display="none"`, data styles, and de and en documents |
| byte-identical round trip | Each fixture read and written untouched is the same bytes. With `GRIND_LO_CORPUS`, the same holds for the 15 corpus documents that hold these fields (loop A, and `never_worse.rs`) |
| an edit changes only what it must | Editing one field rewrites that field and its dependents' values and text, and nothing else in the XML (an R6 diff check) |
| schema | Every written fixture, every inserted field, and both syntaxes, through `jing` (it skips without it, as R2 does elsewhere; `scripts/claude-vm.sh` has it) |
| **the oracle: values match LibreOffice** | Generated documents over the whole translation table and three languages. grind's recalc is compared with `soffice --convert-to pdf` plus extracted text (F10), through the pinned-image shim. Divergences are listed by name: F7, F8, and unknown names |
| loop C | grind writes `ooow:` fields, `soffice` saves them, and grind reads them back with the same formulas and values. `of:` fields are this loop's one new documented loosening (F2): formula gone, value kept |
| cycles and errors | A user-field circle is `#NUM!` and named in words. Downstream fields propagate it. Division by zero is `#DIV/0!`. An unknown name is `#NAME?`. A refused formula keeps its cached value. A seeded random test holds incremental evaluation equal to evaluating from nothing |
| external references | A sibling `.ods`: a cell, a named range, a range inside `SUM`. A missing file, `http:`, `..`, a symlink pointing out of the directory, a text file named `.ods`, and a sheet whose own external reference must not be followed. **Opening and editing never call the resolver** (a resolver that panics) |
| parity and the rest | `cli/tests/parity.rs`, `cli/tests/cli.rs` (sample-text.sh), `cli/tests/local_only.rs` and `cli/tests/simplicity.rs` stay green |

## 8. Commits

1. Facts F1–F10 in `doc/odt-format.md` §5. The field model (a field run replacing `Run::Kept`
   for these six elements), reading and the byte-identical round trip, with fixtures.
2. `ExprList` made public, the document-order mapping, `App::recalc` without sources, the
   `ooow:` translator table with each row measured, display formatting. `grind text recalc`.
3. External references: `Engine::with_sources`, the resolver, and the path policy.
4. CLI: `fields`, `field`, `input`, `@name` addressing, the parity rows, sample-text.sh, and the
   changelog.
5. TUI: formula on focus, editing, `:fields`, `:recalc`; `doc/tui-shell.md` and
   `doc/feature-matrix.md`.

## 9. Questions for review

1. **The syntax rule (§3):** `ooow:` when exact, `of:=` with a warning otherwise, and no option.
   Agreed?
2. **The path policy (§5):** only the document's directory and below. Or should `..`, meaning
   siblings of the folder, be allowed?
3. **Source values:** the sheet's cached values, or recalculate the sheet in memory first
   (still without following its own links)?
4. **Other shells:** a follow-up (my recommendation), or part of this task?
5. **Errors in the file (§4):** `office:value="0"` with the error name as display text. Or
   copy Writer's `DBL_MAX` convention, so Writer itself shows *faulty*?
6. **User fields stay values:** grind never writes a formula into one, matching Writer (F6).
   Agreed?
