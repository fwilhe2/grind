<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The formula workbench — a formula you can take apart, try and put back

**Status: F0, F1 and F2 done.** F0 is this document and `doc/ods-format.md` §4.2, the
measurement of when moving a sub-expression into its own cell changes what a formula means.
F1 is the trace: `parse::parse_spanned`, `display::parse_display`, `formula::trace`,
`App::trace` and `grind sheet eval --trace`, held to loop B's corpus by
`sheet/tests/corpus_trace.rs`. F2 is the *what if*: `trace::Overlay`, `App::overlay` and
`eval --with`. Nothing from F3 on is built. **Experimental**: it ships behind the palette and the CLI, and it
changes no existing verb.

## 1. What it is

An IDE's *evaluate expression*, for a spreadsheet. A pane beside the grid where somebody can:

1. **Discover** a function: search by name or plain-English name, read its signature and
   summary, and try an example that is known to be right.
2. **Try** a formula live: every sub-expression shows what it comes to, at the cell the formula
   would sit in, while it is being typed, with the completion every formula editor already has.
3. **Ask what if**: give any cell the formula reads a different value, for the pane only, and
   watch the answer change. Nothing in the sheet is touched.
4. **Read** a long formula: one expression laid out over several lines, nesting indented, with
   a value at the end of each line.
5. **Put it back**: apply the formula to a cell, or **extract** a sub-expression into a cell (or
   a helper column, or a named expression) so that a nested formula is several readable ones.

### What is new, and what is not

Excel's *Evaluate Formula* dialog steps through a formula one reduction at a time.
LibreOffice's Function Wizard shows a result for each argument and a structure tree. Google
Sheets shows a result chip while you type. So "see what a sub-expression evaluates to" is not
new, and this design does not claim it is.

What none of them has is **inputs that can be changed without touching the sheet** (3) or
**extraction as a refactoring that refuses rather than changes the meaning** (5). Those two are
the point of the feature, and they are where the work is.

## 2. Decisions

1. **No new syntax.** The workbench edits one OpenFormula expression in display syntax: the
   formula bar's grammar, `formula::display`, and no other. There are no variables, no
   statements and no `x = 5` lines. A scratch language would be a language no cell can hold
   (R1), and a step towards the macro line `doc/not-doing.md` §1 draws. When names for
   intermediate values are wanted, the answers are a cell, a defined name, and one day `LET`,
   which waits on `doc/function-coverage.md` §7.
2. **Multi-line means a layout, not a program.** IntelliJ needs a multi-line mode because Java
   has statements. A formula is one expression, so here several lines are the *same*
   expression pretty-printed, with a value at the end of each line. Line breaks and indentation
   are display only. What is applied to a cell is the canonical one-line formula, and a test
   proves the layout parses back to the same tree.
3. **Nothing in the workbench is written into the document.** It holds the scratch formula, the
   overrides and the history as a shell's state, the way `doc/view-modes.md`'s overlays are
   state: open the workbench, try everything, close it, save, and the bytes are identical. The
   two verbs that *do* write, Apply and Extract, are ordinary edits, each one `Action::Batch`
   and so one Ctrl+Z.
4. **What-if inputs are an overlay, and they are honest.** An override replaces a cell's value
   for one evaluation. Every formula that reads that cell, directly or through other formulas,
   is evaluated again under it. A value that is stale because an override did not reach it would
   be worse than no feature at all.
5. **Extract refuses rather than changing meaning.** Moving a sub-expression into a cell is not
   a syntactic operation in ODF. `doc/ods-format.md` §4.2 measured four ways it changes an
   answer. Every one is either refused, or offered with a target that preserves the meaning (§5).
6. **Core first, the CLI reaches every piece, and one shell first.** Every answer the pane shows
   is a core call, every core call has a `grind sheet` command (rule 4, `cli/tests/parity.rs`),
   and the first window is `grind-sheet-gtk`. The other five clients follow the way
   `formula::assist` showed they can: a pane that only draws.

## 3. The core

What already exists and is reused unchanged:

| Need | Already there |
|---|---|
| Evaluate without storing: a read lock, no undo entry, no observer | `App::preview` (`sheet/src/lib.rs`), `verbs::evaluated`, `grind sheet eval` |
| Completion, the call the caret is in, signatures | `formula::assist` (`prefix_at`, `candidates`, `call_at`, `signature_parts`, `brief`) |
| The function list, plain-English names, an unfolded reading | `assist::function_lines`, `friendly::{alias, signature, explain}` |
| Ranking a query | `grind_core::search::score` |
| Re-aiming relative references at another cell | `formula::shift` |
| Substituting one tree into another, bracketed by precedence | `formula::rename` (`inline_name` is extract-to-name run backwards) |

What has to be added, all in `grind-sheet`:

### 3.1 Spans on the tree (F1, built)

The workbench needs every node's byte range in the *display* text: to highlight a node, to put
a value beside it, and to let somebody select a sub-expression by selecting its text. Built as
a parallel `parse::Shape { range, children }` tree, child for child with the `Expr` beside it,
so the `Expr` every other caller matches on is unchanged. The parser records each node's token
range as it builds it (`lex::lex_extents` gives where each token ends as well as starts), and
only when asked: `parse` passes `spanned: false` and builds no children for it, since the
evaluator parses once per formula per evaluation. `display::parse_display` runs
`from_display`'s one scanner and maps the canonical shape back onto the typed text, so a node
that was respelled (`b2` → `[.B2]`, `TRUE` → `TRUE()`) covers exactly what was typed. An
omitted parameter is an empty range where it would have been.

**Held to the corpus**: for all 76,068 formulas of loop B that survive display form, every
node's text cut out and parsed alone is that node's subtree exactly.

### 3.2 `App::trace` (F1, built)

```rust
pub fn trace(&self, sheet: usize, pos: Pos, formula: &str) -> Result<formula::trace::Trace>
// Trace { value: Value, root: Node }   Node { range, came_to: CameTo, children }
// CameTo::Value(Value) | CameTo::Range(Area) | CameTo::Skipped
```

`formula` is display syntax and every range is a byte range in it. The engine keeps a
`Recorder` — each node's operand, keyed by the node, recorded only at the traced formula's own
depth, so another cell's formula and a named expression (both evaluated one deeper) never land
in it. A node it never saw was never evaluated. A one-cell reference is shown as that cell's
value, with an empty cell as `Value::Empty` rather than 0: the trace is the one place a person
can see the difference. `CameTo::spelled` writes an answer as a formula would (`"3"` is not
`3`), and `Trace::rows` is the depth-first list the CLI prints.

Every node of the tree evaluated at `at`, so a sub-expression's value is the one it has *in
place*: a range shows as a range ("40 cells, 10…400") rather than being forced to a scalar, an
error shows as itself, and a branch `IF` did not take is marked *not evaluated* rather than
being evaluated anyway. The last point is what distinguishes a trace from evaluating each
sub-expression in isolation: `IF(B2=0;0;A2/B2)` must not show `#DIV/0!` on a branch that never
ran. It takes the same read lock as `preview`, makes no undo entry and notifies no observer,
and there is a test for each of the three, as `preview` has. **Held to the corpus**: 23,971
traces (the first fifty formulas of each of 879 documents — each builds its own engine, as
the workbench's will) have the evaluator's answer at the root; the four that differ are in
documents calling `NOW()`, counted rather than failed.

### 3.3 `Overlay` (F2, built)

```rust
pub struct Overlay { values: HashMap<Address, Value> }   // set, clear, get, is_empty
pub fn overlay(&self, entries: &[(usize, Pos, &str)]) -> Result<Overlay>   // on App
```

`App::overlay` reads each entry by the typing rule `App::enter` uses for that cell (the `'`
rule, a blank for an empty string, a date cell's ISO spelling, the document's locale), so an
override means what typing it would mean. A value of the formula engine's own kind rather than
a `CellValue`, because an override of *empty* has to stay empty — a cell cannot hold it, but
"what if this were blank" is exactly the question.

This is cheaper than it looks. `Engine::new` already evaluates every formula cell from its
formula, with a memo of its own (`cached: false`), so seeding that memo with the overrides
before evaluation reaches the dependents for free. The engine never reads a cached value it
could recompute, so no dependent can be stale. The cost is that a trace evaluates the
formula's whole precedent cone fresh, which is `preview`'s cost today. Overriding a formula
cell's value is allowed, since that is what "what if this subtotal were 0" means.

### 3.4 The layout (F3)

`display::pretty(canonical, width) -> Vec<Line>`: a call that fits in `width` stays on one
line, and a call that does not puts each argument on its own line, indented, with the closing
parenthesis on its own line. Each `Line` carries the node its value belongs to, so a shell
draws `value_of(line.node)` at the line's end. It is held to two properties: joining the lines
and reading them back gives the same `Expr` (the round-trip test), and the same input gives the
same lines.

### 3.5 `App::extract` (F4)

```rust
pub fn extract(&self, sheet: usize, at: Pos, node: Range<usize>, to: Target) -> Result<Extracted>
pub enum Target { Cell(Pos), Column { from: Pos }, Name(String) }
```

One `Action::Batch`: the helper cell (or column, or name) receives the sub-expression,
`formula::shift`ed from `at` to the helper's position. The formula has the node replaced by a
reference to it, or by the name, bracketed by the printer's precedence rule. The rules for
when to refuse are §5. `Extracted` reports what was written and where, as
`App::rename_name` reports what it carried.

## 4. Discovery

The function browser is `assist::function_lines` (name, plain-English name, categories,
summary) ranked by `search::score`, with the signature from `friendly::signature`. Beyond those
it gives each function **examples**, and an example follows the rule every expectation in this
project follows: **measured, never hand-computed.** Two sources:

- `funcs/beyond.rs`'s fixture and expectations, for the 28 functions beyond the line. They
  were measured against the oracle on 2026-10-12, so these examples already exist. They need
  moving from test code into a table the test and the browser both read.
- For the Small Group, a table of the same shape, measured the same way (§4.1's method). Until
  a function has a row, it shows no example rather than an invented one.

An example is a formula and a fixture. "Try it" loads both into the workbench with the fixture
as an **overlay** over empty cells, so the example runs against the numbers it was measured
against, whatever the sheet holds. This is the same mechanism as what-if, so there is no second
one.

## 5. Extraction — when it is refused

`doc/ods-format.md` §4.2 is the evidence. A sub-expression `S` of a formula at `at` may move to
a target only if `W([.helper])` means what `W(S)` meant. The rules, in the order they are
checked:

| Class | The rule | Why (§4.2, Part 4) | Instead |
|---|---|---|---|
| **Range-valued** | `S` evaluates to a reference to more than one cell | in a cell it becomes an implied intersection (§6.3.3), different in another row and lost entirely as an argument wanting a reference (`SUM`, `ROWS`) | **`Target::Name`**: a named expression holds a range |
| **Position-dependent** | `S` contains a call whose answer depends on where it is evaluated: `ROW()`, `COLUMN()` with no argument, and anything else the catalog marks so | `ROW()` answers the helper's row | refuse; it is one call, and extracting it gains nothing |
| **Sequence argument** | `S` is (or reduces to) a direct Text or Logical, and is in a parameter the catalog types as a sequence (`NumberSequence`, `LogicalSequence`, …) | §6.3.7 converts a direct text or logical and drops a referenced one: `SUM("3")` is 3, and `SUM` over a cell holding `"3"` is 0 | refuse, naming the parameter |
| **Empty** | `S` is a reference, or a call that can pass an argument's value through (`IF`, `IFERROR`, `IFNA`, `CHOOSE`, `INDEX`, the lookups), and some branch can be an empty cell | a cell cannot hold *empty*: it stores 0, and `ISBLANK`, `COUNTBLANK`, `COUNT`, `COUNTA`, `&` and `=""` all change (§4.7) | **refused** when `S` is empty *now*; **warned** when it only could be, saying which function would notice |
| **Cycle** | the helper is a precedent of `S` | a circular reference | refuse |
| **Occupied** | the target cell is not empty | an extract never overwrites | refuse |

Then the **dynamic check**, on every extract the static rules let through: `W` is evaluated
twice, as written and with the node replaced by a reference to the helper under an `Overlay`
that gives the helper what `S` evaluates to *as a cell* (`eval::to_cell`, the same conversion a
real cell goes through). If the two answers differ, the extract is refused and both answers are
shown. The static rules exist because the dynamic check only knows today's values. The dynamic
check exists because the static rules are a list and lists are incomplete.

**A formula filled down a column** is the case that needs a decision: extracting from one row
of a thousand copies. `Target::Column` writes the helper beside every row the same formula
(after `shift`) occupies and rewrites each of them, as one batch. `Target::Cell` refuses with
the count and offers `Column`, because extracting from one row of a filled column breaks the
fill silently.

Not in scope: extracting from a formula in a merged area's covered cell, from an array
formula, or across sheets. Each is refused by name.

## 6. The CLI (R9)

```sh
grind sheet eval   book.fods D2 '=IF([.B2]>0;[.B2]*[.C2];0)' --trace       # every node, indented
grind sheet eval   book.fods D2 '=…' --with B2=-1 --with C2=10             # the overlay
grind sheet eval   book.fods D2 '=…' --pretty --width 60                   # the layout
grind sheet extract book.fods D2 '[.B2]*[.C2]' --to E2                     # or --column E, --name line_total
grind sheet functions --examples ROUND                                     # the measured examples
```

`--trace` prints one line per node in `--pretty`'s layout, with the value aligned at the right,
and `--format json` returns the `Trace` as is. `extract` names the node by its text, the first
occurrence, or `--at <offset>` when the text occurs twice. `doc/cli-parity-sheet.md` gains
`trace` and `extract`. `examples/sample-sheet.sh` gains an `extract` line, as every capability
must.

## 7. The pane

`grind-sheet-gtk` first. Under `doc/sheet-shell.md`'s *Four surfaces* the workbench is not a
toolbar button: it is reached from the **command palette** (*Open Formula Workbench*)
and the cell's **context menu** (*Open in Workbench*, which loads that cell's
formula and position), and it opens as a side pane the grid stays usable beside, so a click on
a cell while it is open inserts a reference, as point mode does.

```
┌ Functions ─────────────┬ Expression at D2 ───────────────────────┬ Inputs ──────────────┐
│ ⌕ round                │ =IF(                              TRUE  │ B2    4      [     ] │
│ ROUND     Round        │     B2 > 0;                       TRUE  │ C2    2.5    [ 10  ] │
│ ROUNDUP   Round Up     │     ROUND(B2 * C2; 2);            40    │                      │
│ ROUNDDOWN Round Down   │     0                       not taken   │                      │
│ ┌─────────────────────┐│ )                                 40    │ was 10 → now 40      │
│ │ ROUND(Number; Count)││                                         │                      │
│ │ ROUND(2.5;0) → 3 ✓  ││ [Apply to D2]   [Extract selection ▾]   │ [Reset inputs]       │
│ │ [Try it]            ││                                         │                      │
└────────────────────────┴─────────────────────────────────────────┴──────────────────────┘
```

- The expression is an editor in display syntax with `formula::assist`'s band under it. Tab
  accepts and Enter inserts a line break, since in a multi-line editor nothing is committed by
  a key. Apply is Ctrl+Enter.
- Selecting text, or moving the caret, selects the smallest node that covers it. The node is
  highlighted, its line's value is emphasised, and the *Extract selection* menu shows the three
  targets with the refused ones greyed and the reason as their tooltip.
- **Inputs** lists every cell the formula reads, its real value, and an override field. The
  footer shows the formula's answer before and after the overrides.
- **It is where *Explain This Formula* grows up.** Ctrl+Shift+E unfolds a formula read-only
  today (`friendly::explain`), which is the workbench's layout without the values, the inputs
  or the editor. The open question for F5 is whether Explain *becomes* the workbench on that
  key, or stays as the quick read-only popover with the workbench one click from it. The
  second is the default: a popover and a pane are different weights.
- Friendly mode (`friendly::explain_inline`) is a toggle on the expression, as in the formula
  bar, and is never what is applied.

The other clients, in the order `formula::assist` reached them: `ui_win32` (a docked pane on
decision 8's grounds, not a popup), `ui_web` (a side panel, Ctrl+K), `ui_tui` (`:workbench`, the
three columns stacked), `ui_mac`. Each gets a row in `doc/feature-matrix.md`.

## 8. Milestones

| | Builds | Done when |
|---|---|---|
| **F0** | This document and `doc/ods-format.md` §4.2 | done, 2026-10-12 |
| **F1** | Spans, `App::trace`, `eval --trace` | done, 2026-10-12: every span over loop B's corpus, the root against the evaluator, and a branch not taken never evaluated |
| **F2** | `Overlay`, `--with` | done, 2026-10-12: an override two formulas away, into a range, on a formula cell and as a blank; the CLI's file unchanged |
| **F3** | `display::pretty`, `--pretty` | Round trip over loop B's corpus: the joined lines parse to the same tree |
| **F4** | `App::extract`, `grind sheet extract`, every row of §5 | Each §4.2 row is a test that is refused, warned or allowed as §5 says. Undo restores the bytes. Loop C holds an extracted document |
| **F5** | The GTK pane, examples moved into a shared table | Rendered by `--render-to`, and the palette and context-menu entries walked by `chrome_tests` |
| **F6** | The other clients | A row in `doc/feature-matrix.md` filled for each |

F1–F3 are useful alone (`--trace` already does more than a step-through dialog) and risk
nothing, because they write nothing. F4 is where the risk is, and the reason F0 measured before
anything was built.

## 9. Open, and named

- **The four divergences §4.2 found in passing**: LibreOffice's third state for an empty
  formula result, `COUNT` of an empty value passed directly, approximate `=`, and a `-0`
  written for an empty `SUM`. They are not the workbench's to decide, but the workbench is the
  first feature that shows them to a user, side by side.
- **`LET`** would let the workbench offer *extract to a local name* without a cell. It needs
  the engine to have scopes. It does not need arrays, which sets it apart from the rest of
  `doc/function-coverage.md`'s third tier.
- **Persistence**: the scratch formula and overrides live for the window's lifetime. Keeping
  them across sessions would need a store that is not the document (decision 3), and none is
  planned.
- **A formula that reads its own cell** is traced, like `App::preview` evaluates it, against
  what the cell holds *now* — the formula is not there yet, so it is not circular yet. Applying
  it would make a cycle the trace did not show; the workbench should say so before Apply
  rather than after (`trace.rs`'s
  `a_formula_that_reads_its_own_cell_sees_what_the_cell_holds_now`).
- **Array values** are absent from the engine, so a trace shows a range as a range and never as
  a matrix of values.
