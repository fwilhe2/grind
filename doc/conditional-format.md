<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Conditional formatting — one rule type

**Status: built (2026-10-05).** Decided 2026-10-04, replacing `doc/not-doing.md`'s
"conditional formatting beyond one rule type" as a reason to have *none*. §3's questions were
measured and written into `doc/ods-format.md` §3.6 before any code, and §4's steps 1–4 are done:
the model (`sheet/src/rule.rs`, `Sheet::rules`, `Action::SetRules`), `App::rules`/`set_rules`/
`add_rule`/`remove_rule` and `grind sheet rule`, the projection's `rule` node, the viewport's
evaluation, the reference index and `grind lint`, ODF read and write with loop C over two rules on
one cell, and the import from `.xlsx` (`xlsx/src/rules.rs`, `doc/xlsx-format.md` §4.12). What
§4's step 5 leaves — a palette verb per shell for *editing* rules — is the open row in
`doc/feature-matrix.md`.

Three things the build settled that §2 left open, each written down where it lives:

- **A rule has several ranges and one base cell**, not `range: (Pos, Pos)`: both formats give a
  rule that shape (Excel's `sqref`, ODF's one `style:base-cell-address` however many cells carry
  the map), and splitting one would respell its condition per piece.
- **The file states the order only where two rules meet on a cell** (the maps' order on a cell
  style). A writer and a reader agree on that, and two rules that never share a cell come back in
  the order their first cell is met (`sheet/tests/rules.rs`).
- **A rule is written on the cells the rest of the sheet spans**, not past them (`Sheet::ruled`,
  a ponytail): `A1:A1048576` is how Excel says "this column", and a million written rows is not
  what anybody meant.

## Why it is in

`doc/plan.md` never excluded conditional formatting. It excluded conditional formatting
*beyond a single rule type*, and `doc/not-doing.md` gave the reason: the general form is a rule
engine with its own evaluation order. That reason stands for the general form. But the one rule
type it left room for was never built, and the "demonstrated need" behind that line was
measured against the wrong corpus. R7's `samples/conditional-formatting.fods` uses LibreOffice's
extension types: a colour scale, an icon set and a data bar. Those make conditional formatting
look like decoration. What real documents use is the plain rule: **this formula is true here,
so draw the cell this way.**

The document that made the case is a checklist exported from an online spreadsheet
(`xlsx/tests/checklist.rs` rebuilds its shape). Each item's tick is a boolean in a hidden
column, and `$E5=TRUE` fills the item's row green. Without the rule, nothing on screen says
which items are done. The rule *is* the checklist.

## 1. The scope line

**In:** one rule type: a condition that is a formula, evaluated with its references relative to
each cell of the range it applies to, and a cell style applied while it is true. An ordered list
of such rules per sheet; the first rule that is true wins (§3, Q4, confirms that is the
oracle's order). The applied style is a `CellStyle`, the same struct a cell already carries, and
it is drawn *over* the cell's own style property by property: a rule that sets only a
background leaves the cell's bold alone.

**Out, and named:**

| Not doing | Because |
|---|---|
| Colour scales, data bars, icon sets | A value-to-colour mapping and two kinds of drawing. They are a second and third rule engine, and LibreOffice spells them only in `calcext:`, which R4 lets in only when it is the sole way to get a measured behaviour |
| The value-comparison rule types (`cell-content()>5`, between, duplicate, top-N, contains-text, date bands) | Each one is a formula: `[.A1]>5`, `COUNTIF(…)>1`. The import filter and the CLI write it as one. A spreadsheet application offers them as shortcuts, and a shortcut is a shell's business. The model does not need a second vocabulary for something the first can already say |
| Stop-if-true, priorities spelled as numbers | Order in the list *is* the priority. A rule after a true rule is not consulted, which is the only stopping behaviour a first-match list has |
| A rule that changes a cell's number format | It is a different style family (§5.2), and its reader would need a second resolution path. The applied style is `CellStyle` only. A rule in a file that carries a format is counted as a loss on import and kept by R6 in a file nobody restyled |

## 2. The model (proposed)

```rust
/// One rule: while `condition` is true at a cell of `range`, `style` is drawn over its own.
pub struct Rule {
    pub range: (Pos, Pos),           // where it applies
    pub condition: String,           // OpenFormula, ODF syntax, relative to `range.0`
    pub style: CellStyle,            // the properties it sets; the rest are the cell's own
}
// Sheet::rules: Vec<Rule>, in priority order.
```

- **Evaluation is derived, never stored.** This follows `doc/view-modes.md`'s rule for roles: a
  stored answer goes stale and a derived one cannot. `App::get_viewport` evaluates every rule
  that intersects the rectangle and hands the shell the *resulting* style. So no shell evaluates
  a formula, and the four shells cannot disagree about which cells are green.
- **Relative references** are shifted by `formula::shift` from `range.0` to each cell. That is
  the function a fill already uses, so `$E5` stays in column E and moves down a row per row.
- **The dependency graph** (`graph.rs`) learns that a rule reads its references. The question
  "what does this cell affect" then includes the cells a rule draws. That is what lets `grind
  lint` and the role overlay see a hidden column that only a rule reads, which is otherwise a
  textbook "unused input".
- **Undo** is one `Action::SetRules { sheet, rules }`. The whole list is replaced, as a chart is,
  because reordering, inserting and editing a rule each invalidate its neighbours' priorities.
- **The projection** gets a `rule` node: `rule B5:D9 "of:=[.$E5]=TRUE()" background="#97e8ca"`,
  with `doc/projection-sheet.md`'s row and an example the scope test executes.
- **The CLI** gets `grind sheet rule <range> <formula> [style flags…]`, `--clear`, and with no
  range, the list. The style flags are `grind sheet style`'s own.

## 3. What must be measured first (clean room)

**Progress (2026-10-05):** Q1 and Q2 are answered and written up in `doc/ods-format.md` §3.6 — LibreOffice writes *and* reads the standard `style:map`/`is-true-formula`/`style:base-cell-address` spelling, so the reader needs no `calcext` path. Q3 and Q4 are answered too: a rule is (condition, style, base cell), LibreOffice merges the cells carrying the same one into one range on read, and the first true rule wins. §3 is closed.

Each of these is answered from LibreOffice's *behaviour* (write a file, convert it, read it
back) or from the ODF specification. The answers go into `doc/ods-format.md` with a
`MEASURED`/`SPEC` mark before any code depends on them.

1. **The standard spelling.** ODF has `style:map` with `style:condition`,
   `style:apply-style-name` and `style:base-cell-address` on a cell style (rng:12119). The
   last one is what makes `$E5` relative. Q: does LibreOffice *read* a range rule spelled that
   way, from a cell style shared by every cell of the range? Does it *write* one? Which
   condition grammar does it accept (`is-true-formula(…)` and `cell-content-is-…`, as the ODF
   1.2 spec's prose names them)? The specification's own grammar for `style:condition` is not
   in this repository, and has to be read from the published Part 3 before anything relies on
   it.
2. **LibreOffice's own spelling.** It writes `calcext:conditional-formats` for every rule, the
   plain type included (R7's sample). Q: does it *also* write the `style:map` form, so that
   reading only the standard spelling still finds LibreOffice's rules? If not, the reader takes
   `calcext:condition` for this one type: R5 says LibreOffice's files read. The writer still
   writes only the schema-valid form, since R2 outranks R4. Loop C decides whether that
   survives a round trip.
3. **A rule over a range whose cells already carry different styles.** `style:map` lives on a
   style, so N different base styles need N mapped copies. Q: does LibreOffice merge them back
   into one range rule, or keep N rules? This is the R6 and R3 cost of the standard spelling,
   and it has to be measured rather than assumed.
4. **Priority.** Q: is the first true rule the one drawn, when two rules on one cell are both
   true? This is ODF's order for `style:map`, and the oracle's has to match.
5. **Excel's `dxf`.** On import, a `<cfRule type="expression">` with a `dxfId` maps onto `Rule`
   directly; the `dxf` is a partial `CellStyle`. `doc/xlsx-format.md` gets the notes. Every
   other `cfRule` type stays a counted `Dropped::ConditionalFormat`, except a value comparison
   that can be written as a formula, which is translated (§1).

## 4. Order of work

1. §3, measured and written up. **No code before this.**
2. Model, `App`, CLI, projection, loop F. Viewport evaluation and the role overlay.
3. ODF read and write, both spellings in, the standard one out, with loop C over a case that
   has two overlapping rules and a relative reference.
4. Import: `expression` rules from `.xlsx`, held by `xlsx/tests/checklist.rs`. Its
   `the_conditional_rules_are_counted_not_carried` flips, and the ooxmlgen manifest's
   `ConditionalFormat` counts are recorded in `DECIDED_OTHERWISE` where they now differ.
5. Shells: nothing to build. They draw the style the viewport hands them, and the
   *rule-editing* UI is a palette verb per shell, `doc/feature-matrix.md`'s next row.
