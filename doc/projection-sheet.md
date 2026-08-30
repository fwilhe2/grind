<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The projection's grammar — `grind sheet`

> **1:1 with the subset this build supports — and the subset is still evolving.**
> — `doc/dsl.md` §3.7

`doc/dsl.md` §3.7 names the engineering problem this file exists to solve: *a hand-written
grammar is a second scope line, and two scope lines diverge.* The projection is bijective or it
is a prettier `.csv`, and a bijection that nobody checks decays the first time the model grows
a field.

So the grammar is not prose. `sheet/tests/projection_scope.rs` reads this document and fails
the build when it and the code disagree, four ways:

1. **Every node here is one the writer emits.** Extracted from `sheet/src/projection/write.rs`,
   both directions — a node documented and never written is a promise, and a node written and
   never documented is the drift this file exists to catch.
2. **Every example here parses.** The `Example` column is a complete projection body; the test
   puts a `grind spreadsheet` header on it and reads it. An example that stops parsing is a
   grammar change nobody wrote down.
3. **Every example still holds its node after a round trip.** Read the example, project the
   model it produced, and the node has to be in the output. This is what separates *accepted*
   from *carried*: a node the reader parses and then throws away would pass (2) and fails here.
4. **Every field of the model is accounted for** — see *The state* below. A new side table on
   `Sheet` fails the build until it has a node or a named gap, which is the check that would
   have caught charts.

One per application, like `doc/cli-parity-sheet.md` and for the same reason (**R9**): a
`doc/projection-text.md` arrives with D2 rather than a second section here, because two
applications sharing one document would pass every check while covering half as much.

## The nodes

`Carries` is the model state the node is the spelling of. `Example` is a complete projection
body — one or more document-level nodes — and it is executable, not illustrative.

| Node | Carries | Example |
|---|---|---|
| `sheet` | `Document::sheets`, and `Sheet::name` as its argument | `sheet Sales { }` |
| `null-date` | `Document::null_date`, as `YYYY-MM-DD`. Omitted at its default, which is why the example is the *other* epoch a real document uses | `null-date "1904-01-01"` |
| `null-year` | `Document::null_year`. Omitted at its default | `null-year 1919` |
| `name` | one entry of `Document::names` — §5.11's `table:named-expressions` | `name "tax_rate" "[$Sales.$B$1]"` |
| `col` | `Sheet::col_widths` and `Sheet::hidden_cols` for one column, numbered from 1 | `sheet S { col 2 width="2.258cm" hidden=#true }` |
| `row` | two things by position: `Sheet::row_heights` and `Sheet::manually_hidden_rows` at the sheet level, and one line of a grid inside an `at` | `sheet S { row 3 height="0.45cm" hidden=#true; at A1 { row Region 4200 #null } }` |
| `at` | where a grid of plain values starts. Its rows carry `Sheet`'s cell values; `#null` is a hole | `sheet S { at B2 { row 4200 4800 } }` |
| `cell` | one cell that carries more than a value — `Sheet::formulas`, `Sheet::kinds`, and the value beside them | `sheet S { cell B5 15400 formula="of:=SUM([.B2:.B4])"; cell A7 45123 date=#true; cell A8 0.5 time=#true }` |
| `style` | one entry of `Sheet::styles`, over every cell of its range | `sheet S { style B1:C1 bold=#true italic=#true size="12pt" color="#ff4136" background=navy align=center valign=middle wrap=#true border="0.5pt solid #000000" }` |
| `format` | one entry of `Sheet::formats`, over every cell of its range. Compact when `numfmt::preset` builds it, a block of parts when it does not | `sheet S { format B2:C5 currency decimals=2 grouping=#true symbol="EUR" locale="de-DE"; format D1 datetime }` |
| `filter` | `Sheet::filter` — §9.4's `table:database-range` | `sheet S { filter "__Anonymous_Sheet_DB__0" A1:C4 header=#true buttons=#true }` |
| `keep` | one field of `Filter::keep`: the values that field keeps | `sheet S { filter "f" A1:C4 { keep 0 North South } }` |
| `map` | one `numfmt::Map` — a `style:map` branch, its comparison, its operand and its own format | `sheet S { format A1 number { map ">=" "0" number { number decimals=2 } } }` |

### The parts of a number format

`numfmt::Format` is an ordered sequence of `Part`s and not a format string, deliberately
(`doc/ods-format.md` §5.2), so the projection spells the parts rather than inventing Excel's
`#,##0.00`. One node per `number:*` element, named as that element is minus its prefix.

Each example below is deliberately **not** one of `numfmt::preset`'s, because a preset is
written in the compact form and its parts would not appear at all — which is check (3) above
doing its job while this table was being written.

| Node | Carries | Example |
|---|---|---|
| `text` | `Part::Text` — `number:text`, a literal separator or unit | `sheet S { format A1 number { text " €" } }` |
| `number` | `Part::Number` — `number:number` and its four attributes | `sheet S { format A1 number { number decimals=2 min-decimals=1 min-int=1 } }` |
| `currency` | `Part::Currency` — `number:currency-symbol`, the symbol as its argument | `sheet S { format A1 currency { currency "EUR" } }` |
| `year` | `Part::Year` — `number:year`, `long` being four digits | `sheet S { format A1 date { year long=#true } }` |
| `month` | `Part::Month` — `number:month`; `textual` is a name rather than a number | `sheet S { format A1 date { month long=#true textual=#true } }` |
| `day` | `Part::Day` — `number:day` | `sheet S { format A1 date { day long=#true } }` |
| `day-of-week` | `Part::DayOfWeek` — `number:day-of-week` | `sheet S { format A1 date { day-of-week long=#true } }` |
| `hours` | `Part::Hours` — `number:hours` | `sheet S { format A1 time { hours long=#true } }` |
| `minutes` | `Part::Minutes` — `number:minutes` | `sheet S { format A1 time { minutes long=#true } }` |
| `seconds` | `Part::Seconds` — `number:seconds`, with sub-second decimals | `sheet S { format A1 time { seconds long=#true decimals=2 } }` |
| `am-pm` | `Part::AmPm` — `number:am-pm`, whose presence is what makes the hours a 12-hour clock | `sheet S { format A1 time { am-pm; hours } }` |
| `boolean` | `Part::Boolean` — `number:boolean` | `sheet S { format A1 boolean { text "= "; boolean } }` |
| `content` | `Part::Content` — `number:text-content`, where the string goes in a text style | `sheet S { format A1 text { text ">"; content } }` |

## The state

Every field of `grind_sheet::model::Document` and `grind_sheet::model::Sheet`, and the node
that carries it — or `gap:` and the reason there is none. Read out of `sheet/src/model.rs` at
compile time, so a field added there fails the build until it appears here.

This is the sheet's answer to §3.7's *"an element that enters the scope line without a
projection node fails the build"*. A spreadsheet has no element scope line the way
`doc/text-core.md` is one for text — what it has is a model, and the model's fields are the
thing that grows.

| Field | Node |
|---|---|
| `Document::sheets` | `sheet` |
| `Document::names` | `name` |
| `Document::null_date` | `null-date` |
| `Document::null_year` | `null-year` |
| `Document::source` | gap: R6's retained bytes of the form the document was *read* from. Not state a document has — a physical form does — and a projection is a third form, which D5 gave a splice of its own |
| `Document::projection_source` | gap: **that splice** — the projection's own retained text and the byte range of every cell in it. The same argument one row up, and the reason it is a second field rather than a variant of the first is that the two retain different things (`grind_core::projection::source`) |
| `Document::edits` | gap: what has changed since the read, which is a fact about a session rather than about a document |
| `Sheet::name` | `sheet` |
| `Sheet::cols` | `at` and `cell` — the column store is the cell values, and those are what a grid row and a `cell` node hold |
| `Sheet::formulas` | `cell` |
| `Sheet::kinds` | `cell` |
| `Sheet::formats` | `format` |
| `Sheet::styles` | `style` |
| `Sheet::col_widths` | `col` |
| `Sheet::row_heights` | `row` |
| `Sheet::hidden_cols` | `col` |
| `Sheet::manually_hidden_rows` | `row` |
| `Sheet::filter` | `filter` |
| `Sheet::charts` | gap: `doc/dsl.md` §3.8 — expressible, verbose, and nobody hand-writes one, so charts go in for bijectivity rather than for authoring and are not in yet. Loop F excludes them **by name**, and `charts_are_the_one_named_gap` fails the day they stop being a gap |

## What this document is not

It is not the *design* — `doc/dsl.md` is, and it outranks this file on every question of why.
It is not a specification anyone else should implement against; it is this build's third
serialisation (§9). And it is not a place to record what the projection *will* spell: a row
here is a node that exists, and a gap is a row in the table above with a reason on it.
