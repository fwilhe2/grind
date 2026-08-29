<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# View modes — what a document *means*, drawn

**Status: proposed. Nothing here is normative yet.** Two features, written as one document
because they are one mechanism, one core API, one CLI shape and one verification story:

- **Part I — inline names.** A cell that a name is bound to shows that name, so a model does
  not need a label cell beside every constant. IntelliJ's inlay hints, for a grid.
- **Part II — roles.** Every cell has a *role* the document already implies — input,
  computed, cross-sheet, unnamed constant — and a view mode paints it. Excel makes the author
  apply "Input" and "Calculation" styles by hand; a tool that can derive them should.

Neither writes anything to the document. That is §1, and it is the decision the rest follows
from.

These are shell milestones rather than a phase: they attach to the shells that already exist,
they do not depend on `doc/dsl.md`, and they compose with it — the projection's code view
(`doc/dsl.md` §6) can carry the same overlay, because it reads the same core answer.

---

## 1. The one decision: derived, never stored

**Nothing here is written to a `.fods`, a `.ods` or a `.grind`.** Not as a cell style, not as
`calcext:`, not as a settings block. Five reasons, in the order they bind:

1. **R3.** Nothing is written that the document does not use. A role is not a property of the
   document; it is a reading of one.
2. **R6.** Writing must change as little XML as possible. A view mode that stamped a style on
   every classified cell would be the single largest diff this build can produce, on a
   document nobody edited.
3. **`doc/text-core.md`'s principle, already stated:** *generated content is not content.* A
   role is derived from the formulas and the names, exactly as a table of contents is derived
   from the headings.
4. **A stored classification goes stale, and a derived one cannot.** This is the product
   argument, and it is the strongest one. Excel's `Input` and `Calculation` styles are applied
   by hand: turn an input cell into a formula and it still says Input, in blue, for ever.
   Nobody notices, because the colour was right when it was applied. **Deriving it means the
   colours are a fact about the document rather than a memory of one** — which is the whole
   reason to build this instead of shipping a palette of named styles.
5. **A document a shell opens read-only must render identically.** If the mode wrote anything,
   looking at a file would change it.

The consequence to state honestly: **a document sent to somebody without `grind` arrives
uncoloured.** That is a real loss and it has a gate in §7, not a workaround.

---

## 2. The mechanism: overlays on the viewport

Rule 1 says reads go through `App::get_viewport` and no getter hands out the whole document.
Rule 3 says the core pushes and shells never poll. Both apply unchanged, and they decide the
shape.

`Viewport` already carries three parallel row-major vectors — `cells`, `texts`, `styles` —
each with a doc comment saying why it is carried there rather than fetched per cell: *a
renderer that had to ask per cell would either take the lock once per cell or keep its own
copy of the document.* Overlays are a fourth and a fifth, on exactly that argument:

| Field | Shape | Filled when |
|---|---|---|
| `roles: Vec<CellRole>` | row-major, one per cell, total | the role overlay is requested |
| `names: Vec<(Range, String)>` | the name anchors *intersecting* the viewport | the name overlay is requested |

`names` is a list rather than a per-cell vector because a name binds to a **range** as often as
to a cell, and `sales` over `A2:A50` is one anchor, not forty-nine.

Requested, not always computed: `get_viewport` gains an `Overlays` argument — a small set of
flags — so a shell that draws neither pays for neither, and the CLI asks for exactly what it
prints.

**The precedent this follows is already in the codebase.** `formula::display::spans` exposes
the reference scanner in byte ranges *specifically* so that the in-cell editor's colourer and
its committer cannot disagree about what a reference is — "there is one scanner". Part II is
that idea moved from the formula bar to the grid: **one classifier, in the core; four shells
that choose colours and nothing else.**

### Where the analysis happens

A role like *nothing references this cell* is a document-wide question, and answering it inside
a viewport read would be O(document) per scroll. So:

- The **analysis** is document-wide, lives in the core, is computed lazily and cached, and is
  invalidated by `App::mutate` — the same place observers are notified.
- The **read** is viewport-shaped. Rule 1 is about what leaves the core, not about what the
  core may know.

*ponytail:* the first cut recomputes the whole analysis on any mutation. Fine for documents a
person edits; the ceiling is a generated model where a keystroke re-walks a hundred thousand
formulas, and the upgrade is invalidating only the cells downstream of the edit — which needs
§4.4's graph anyway.

---

## 3. Part I — inline names

### 3.1 What a hint is, and what it is anchored to

`Document::names` is a `BTreeMap<String, String>`: a name to the expression it stands for. A
hint exists when that expression is a **plain reference** — a cell or a range and nothing else.

| The name's expression | Hint |
|---|---|
| `[.B2]` | on `B2` |
| `[.A2:.A50]` | on the range `A2:A50`, once |
| `[.Rates.B2]` | on `Rates!B2` |
| `[.A1]*2` | **none** — it is a computed name, anchored to nothing |
| `SUM([.A1:.A9])` | **none**, same reason |

A computed name is not a failure to render; it is a name that does not denote a place. The
formula bar still shows it (§3.3), and the grid has nowhere to put it.

### 3.2 How it is drawn

Inside the cell, at the end opposite the value, in the muted weight every shell already has for
secondary text — never in the gutter, because a gutter would have to grow a column and a hint
belongs to the cell rather than to the row.

Two rules that keep it from becoming noise:

- **The value never yields.** When both do not fit, the *hint* is elided, then dropped. A
  spreadsheet whose numbers are cut off to make room for their labels is worse than one with
  no labels.
- **A range hint is drawn once**, at the range's first visible cell, with the range's border
  marked for the rest — the same treatment a merged region gets. Forty-nine copies of `sales`
  is not a hint, it is a wallpaper.

### 3.3 Names inside formulas — the larger half

The grid hint is the visible half; this is the useful one. `=[.B2]*[.B7]` renders in the
formula bar as `=B2*B7` today. With the overlay on it renders as:

```
=tax_rate * subtotal
```

**This is not a second grammar and must not become one.** `formula::display` already parses and
re-serialises through the canonical printer, and `from_display` scans and re-brackets on the way
back. Name substitution is one more option on that renderer: where a reference exactly equals a
name's expression, print the name. `from_display` already resolves a bare identifier that is not
cell-shaped as a name, so the reverse direction exists.

The check falls out of loop B for free: `display` round-trips 75845 corpus formulas today, and
the substituted form must round-trip through the same assertion.

### 3.4 The line this must not cross

`doc/not-doing.md` §1 excludes **quoted labels and automatic intersection (§5.10)** — by name,
and with the sharpest reason in that table: *the feature that makes a spreadsheet's meaning
depend on where a formula sits.*

So a hint comes from a **declared name and nothing else**. It is never inferred from the text
in the cell above, or to the left, or from a table header. That inference is what §5.10 does,
and building the *display* half of it would teach every user to expect the *evaluation* half.

**Stated as a rule:** if `grind sheet name` cannot create it, no hint shows it.

### 3.5 The trap this walks into, named

`model.rs:258` is a documented limit: named expressions are **one flat map, so a sheet-local
name is visible document-wide.** A hint makes that limit visible for the first time — a name
declared for `Sheet1.B2` would, read naively, hint on `Sheet2.B2` as well.

It must not. A hint is anchored to a **fully-qualified** position: an expression with no sheet
in it anchors to the sheet the name was declared on, and to no other. That is a decision this
feature has to make and a step towards fixing the limit rather than around it.

### 3.6 The text document's version of the same feature

`grind text` has bookmarks, and `#intro` is `loc.rs`'s named address — the exact analogue of a
named range. The same overlay shows a bookmark's name where it anchors, which is otherwise
invisible: today a bookmark is a zero-width `Run` a reader cannot see at all.

Cheap, symmetric, and it makes this a suite feature rather than a spreadsheet one.

---

## 4. Part II — roles

### 4.1 The roles

A role is **total and disjoint**: every non-empty cell has exactly one. That is what makes the
mode legible — a wash with gaps in it reads as a bug.

| Role | Derived from | Convention |
|---|---|---|
| **Input, named** | a literal, with a name bound to it | the good case |
| **Input, unnamed** | a literal nothing else reads | ordinary data — a table of numbers is this, and it is not a problem |
| **Constant, unnamed** | a literal that **at least one formula references**, with no name bound | **the magic number.** §4.2 |
| **Computed, local** | a formula whose references are all in this sheet | |
| **Computed, cross-sheet** | a formula referencing another sheet | |
| **Label** | text nothing references | the row and column headings a person writes |
| **Error** | the value is an error | |
| **Stale** | the cached value disagrees with re-evaluation | already counted by `grind sheet recalc` |

The four-way split of literals is the feature. "Blue for inputs, black for formulas, green for
another sheet" is the financial-modelling convention this borrows, and the reason it is worth
borrowing is that a person reading a model wants to know **what they are allowed to change** —
which is exactly the input/computed line.

### 4.2 A magic constant, defined precisely

The user-facing claim is *"a constant with no name"*, and the definition has to be tighter than
that or every number in a data table lights up:

> A cell is a **magic constant** when it holds a literal, **at least one formula references
> it**, and no named expression is bound to it.

Being referenced is what makes it structural. A column of measurements nobody computes with is
data; a lone `0.2` that three formulas multiply by is a decision somebody made and did not write
down. The fix is `grind sheet name tax_rate B2`, and the hint from Part I then shows it — which
is why these two features belong in one document. **Part II finds them and Part I is what fixing
one looks like.**

Deliberately *out of scope for now*: a literal inside formula text, as in `=[.B2]*0.2`. It is
arguably the worse smell, and it is a different analysis — over the AST rather than over the
grid — with a different fix. `grind lint` (`doc/dsl.md` §4.3) is where it belongs, later.

### 4.3 Roles are not diagnostics, and conflating them ruins the view

Two layers, from one analysis pass:

- **Roles** are neutral and total. A formula cell is not a problem.
- **Diagnostics** are sparse and are problems: stale value, error, a formula reading an empty
  cell, a reference to a deleted sheet, a magic constant.

If the mode paints both in one channel, an ordinary model looks like a wall of warnings and
people turn it off in a week. So roles get the **fill or the text colour**, diagnostics get a
**mark** — a corner triangle, one per cell, in the semantic colour. Same distinction a code
editor makes between syntax colouring and a squiggle, and for the same reason.

The diagnostic half is `grind lint`'s sheet rules, already listed in `doc/dsl.md` §4.3. One
analysis, two surfaces: a list from the CLI, a mark in the grid.

### 4.4 The dependency graph this finally justifies

*Referenced by at least one formula*, *nothing reads this*, and *what changed downstream of my
edit* are all **reverse dependency** questions, and this build has no reverse index:
`formula/eval.rs` recurses over the dependency graph rather than sorting one, and
`doc/plan.md`'s `graph.rs` is *in the plan and unbuilt on purpose*.

**This is the use that pays for it.** And it is not the only one — `doc/not-doing.md` §2 gates
**incremental recalculation** on "when a UI makes whole-document recalc feel slow", and that
needs the same structure. Building it once serves both, which is the evidence gate those rows
ask for.

Scope discipline: a forward index of *which cells each formula reads* is one walk of the
already-parsed ASTs; the reverse index is its transpose. Nothing here needs cycle detection,
topological ordering or incremental maintenance — `eval.rs` keeps doing what it does. **A first
cut is a `BTreeMap<Pos, Vec<Pos>>` built on demand and thrown away on mutation**, and the
milestone succeeds without touching the evaluator at all.

### 4.5 How it is drawn — a mode, not a decoration

**Decided: in role mode, colour means role, exclusively.** The document's own fills and text
colours are suppressed while it is on, and a cell that carries its own styling is marked so
nothing is hidden silently.

The alternative — layering role colour over document colour — was rejected, and the reason is
concrete rather than aesthetic: `ui_sheet_gtk`'s rule is that *every colour comes from the
theme, never a literal, except the reference palette and a colour the document itself chose*.
A wash over a document that already chose its colours produces cells whose colour has two
causes and no way to tell them apart, which is worse than either alone. A **mode** has one
meaning at a time and can be toggled off in one keystroke.

Colours come from `style::PALETTE` — the seventeen clrs.cc colours the project already
describes as *a default a shell offers and never a limit* — resolved against the theme, so the
mode works on a dark ground.

### 4.6 Colour is not the whole of it

This feature's entire output is colour, which makes accessibility a requirement rather than a
courtesy:

- **A second channel per role**: a one-glyph marker in the cell corner, so the mode is usable
  with no colour discrimination at all.
- **`Accessible::announce`**, which `ui_sheet_gtk` already calls on every selection move (and
  is why its `gtk4` feature is `v4_14`). Moving onto a cell in role mode announces the role.
- **The CLI is the accessible surface of last resort**, and §5 is not optional for that reason.

---

## 5. The CLI — R9, and it is not a formality here

Rule 4 has no exception for something that feels like a view. Both overlays are core answers,
so both print:

```sh
grind sheet view book.fods A1:D20 --names     # values, with the name bound to each
grind sheet view book.fods A1:D20 --roles     # the role of each cell instead of its value
grind sheet view book.fods A1:D20 --formulas  # falls out: the source, not the value
grind --format json sheet view book.fods A1:D20 --roles   # machine-readable
grind sheet lint book.fods                    # the diagnostic half (doc/dsl.md §4.3)
grind text view report.fodt --names           # bookmark anchors, §3.6
```

`--format json` already exists at suite level, so a role table is scriptable the day it lands —
which is what makes "find every magic constant in this repository" a shell loop rather than a
feature request.

`doc/cli-parity-sheet.md` and `doc/cli-parity-text.md` gain a row each, and
`cli/tests/parity.rs` enforces them as it does everything else.

---

## 6. Verification — and why loop C is not it

**Every other feature in this project is checked by loop C, and this one cannot be**, because
loop C round-trips through LibreOffice and this feature writes nothing for LibreOffice to see.
That is not a gap; it is the point. So the checks are chosen to match the claim:

| Check | What it asserts |
|---|---|
| **The writes-nothing test** | Open every R7 document, request every overlay, read viewports across the whole sheet, save — and assert the bytes are **byte-identical** to the input. This is the headline check: the feature's entire promise is that it changes nothing, and this is that promise, mechanically |
| **Totality and disjointness** | Every non-empty cell in the corpus gets exactly one role. A property test over loop A's 359 documents |
| **Roles agree with the formulas** | A cell classified *computed, local* has a formula whose every reference is local — checked against loop B's 75845 already-parsed formulas, at zero corpus cost |
| **Name substitution round-trips** | `display` with names on, re-parsed, yields the same AST — loop B's existing display assertion, with the option set |
| **Layout arithmetic** | Hint elision, the range-anchor position, the corner marker — in `ui_sheet_gtk/src/geom.rs`'s shape: arithmetic with no toolkit in it, so no display is needed |
| **`--render-to`** | A role-mode frame comes back byte-identical after a refactor, as every other grid change already proves itself |

Three of those reuse corpora that already exist and already run. That is the cheapest
verification story of any feature in this project, and it is cheap for the same reason the
feature is: **nothing new is being represented, only read.**

---

## 7. What this will not do

| Not doing | Because |
|---|---|
| **Write any of it into a document** | §1. Not as a style, not as `calcext:`, not as settings |
| **Infer a name from a neighbouring label** | `doc/not-doing.md` §1's §5.10 exclusion. If `grind sheet name` cannot create it, no hint shows it (§3.4) |
| **A user-defined rule engine** | "Colour a cell when *my* predicate holds" is conditional formatting, which the document already does and which `doc/not-doing.md` caps at one rule type on purpose. The roles are a fixed, named set |
| **A user-editable role palette** | Same line `style::PALETTE` already draws: a default a shell offers, not a preferences page. The set is small and its meanings are conventional |
| **Hints for computed names** | A name that is not a plain reference denotes no place. The formula bar shows it; the grid has nowhere to put it (§3.1) |
| **Guess at the *intent* of a constant** | `0.2` is not classified as a tax rate. The feature says *this is unnamed*, and naming it is the author's sentence to write |

### Not yet, with a gate

| Not yet | Gate |
|---|---|
| **Baking the roles into a document** — `grind sheet bake-roles`, one explicit verb writing real cell styles | When somebody has to send a coloured model to a person without `grind`. It is a *conversion*, must be spelled as one, and must never be what the view mode does implicitly — the moment it is automatic, §1's point 4 is back and the colours can go stale again |
| **Magic literals inside formula text** (`=[.B2]*0.2`) | `grind lint`, `doc/dsl.md` §4.3. A different analysis over a different structure, with a different fix |
| **Roles in the projection's code view** | `doc/dsl.md` D9. It reads the same core answer, so this is a rendering question in one shell rather than a feature |

---

## 8. Milestones

| | What | Done when |
|---|---|---|
| **V0** | This document. The role set is fixed and the overlay API is designed | The names are settled |
| **V1** | The forward and reverse reference index (§4.4) — one walk of the parsed ASTs, built on demand | It agrees with `eval.rs` about what every corpus formula reads |
| **V2** | `CellRole`, the classifier, `Overlays` on `get_viewport` | Totality, disjointness and the formula-agreement checks pass over loops A and B |
| **V3** | `grind sheet view --roles` / `--names` / `--formulas`, `--format json`, parity rows | The CLI can answer everything before any shell draws it (R9, and §4.6's accessibility floor) |
| **V4** | Name anchors — the grid hint and the range case (§3.1–3.2) | The writes-nothing test is green, and `geom.rs` covers elision |
| **V5** | Name substitution in `formula::display` (§3.3) | Loop B's display round-trip passes with the option on |
| **V6** | Role mode in `grind-sheet-gtk` — the mode, the palette, the corner marks, `announce` | `--render-to` frames are stable; the a11y second channel is there from the first commit, not after |
| **V7** | The other three shells, and `grind text --names` for bookmarks (§3.6) | Each shell's gap list is updated rather than each shell inventing colours |

V1–V3 are the feature and they need no shell at all. V4–V7 are four renderings of an answer
already computed.

---

## 9. Risks, honestly

**The analysis is document-wide and the read is viewport-shaped, which is a cache with an
invalidation rule.** Caches with invalidation rules are where correctness goes to die. The
mitigation is that the rule is trivial — invalidate on `App::mutate`, in the one place
observers are already notified — and the first cut recomputes everything, so the only bug
available is a stale cache, not a wrong one.

**"Magic constant" will produce false positives and they will be annoying.** A referenced
literal that genuinely needs no name — a `1` in a counter, a `0` — will light up. The mitigation
is that §4.3 puts it in the *diagnostic* channel rather than the role wash, and diagnostics can
be dismissed per document. If they cannot be, this rule becomes a lint the user turns off, and
that is a worse outcome than not shipping it.

**Suppressing the document's own styling in role mode will surprise people.** It is the right
call (§4.5) and it is still a surprise. The mode needs an obvious, always-visible indication
that it is on — not a menu checkmark somewhere.

**Colour-only meaning excludes people, and this feature is nothing but colour.** §4.6 is
therefore load-bearing, and the marker glyphs have to ship in V6 rather than being scheduled
after it. A feature that is unusable without colour discrimination and ships anyway does not
get fixed later.

**The flat name namespace (§3.5) may make hints wrong before it makes them useful.** The
mitigation is to anchor on a fully-qualified position from the first commit and to treat the
underlying limit as something this feature *exposes* rather than something it may reproduce.

**And the reason to do it, stated plainly.** Every spreadsheet in the world is a program whose
structure is invisible: you cannot see which cells are inputs, which are derived, or which
numbers somebody typed once and forgot. The conventions exist — financial modellers colour
their inputs blue by hand, and Excel ships styles for it — and they are all *manual*, which
means they are all eventually wrong. Deriving them is a small feature that makes a spreadsheet
readable, and it is available to this build because the core already knows the formulas, the
references and the names. It writes nothing, breaks nothing, and can be turned off with one
key.
