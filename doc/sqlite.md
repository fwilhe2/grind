<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# SQLite in grind — a query layer, a history store, a workspace index

**Status: design, unbuilt, unscheduled.** Not normative for anything. Phase 11 is spoken for
(`doc/xlsx-import.md`) and `doc/dsl.md` has first claim on phase 12; this is a candidate behind
both, written down so the decision it turns on is decided once rather than re-argued each time
someone reads <https://sqlite.org/appfileformat.html>. `doc/plan.md`'s requirements and
`doc/not-doing.md` outrank every word of it, and §2 is the argument that none of it contradicts
either.

The prompt was *"SQLite as an alternative projection method"*. The answer to that exact question
is no, and §1 says why in one paragraph. The interesting document is the one that starts after
the no: SQLite's real strengths are queryability, atomic incremental writes and a relational join
engine, and **grind has one place where all three are wanted and three places where one of them
is** — which is the shape of this document.

The load-bearing part is **Part I, the query layer**, and its argument is not "spreadsheets would
be nicer with SQL". It is that `grind` is already an **ingest path for other people's data** —
CSV import is built, `.xlsx` import is scheduled — and the step that is missing after ingest is
the one relational engines exist for.

---

## 1. The one decision: SQLite is never a document format

A document has three physical forms and gains no fourth: the package (`.ods`/`.odt`), the flat
file (`.fods`/`.fodt`), and the projection (`.grind`). `Form` is a three-armed enum on purpose.

The appfileformat argument is a good one and it is an argument for *applications whose documents
are not text*. This project's third form exists because the first two are not diffable, and
`doc/projection-guide.md` §1 is a whole section on why that matters. A SQLite file is the exact
opposite property: `git diff` says `Binary files differ`, `grep` finds nothing, a merge conflict
is unresolvable, and a reviewer cannot read a pull request. Adding it as a document format would
be adding back the thing the third form was built to remove, and it would do it while also
costing binary size on a project that measures its own (3.6 MB for `grind`, README §"Size").

There is a second reason, and it is the one that would still hold if diffability were not a
goal. A live relational store *invites* mutation from outside the application — that is its
appeal — and every mutation grind makes has obligations attached: an inverse for undo
(architecture rule 2), a splice or a regenerate (R6), a loop C round-trip, a loop F projection.
A store anyone can `UPDATE` has none of those and cannot be given them.

**So: nothing below stores a document in SQLite.** Everything below is either *derived from* a
document and thrown away, or *beside* one and disposable.

## 2. The rules this has to survive

| Rule | Where | How each part stays on the right side |
|---|---|---|
| **R1 — independence, ODF-native semantics** | `doc/plan.md` | SQL never becomes a second formula language. The query layer reads a model that is already built; it does not evaluate cells, and a query result is not a cell value until a person asks for it to be written as one (§3.6) |
| **R3 — minimal boilerplate**, and the size budget | `doc/plan.md`, README | Its own crate behind a cargo feature, exactly as `sheet-xlsx` is (`doc/xlsx-import.md` §"Crate layout"). A build without the feature contains no SQLite |
| **R11 — no crate that opens a document may depend on the evaluator** | `doc/dsl.md`, `build/tests/manifest.rs` | Unchanged, and this document proposes its sibling: **no code path that opens a document may open a database.** See below |
| **"External data sources" — §1 Never** | `doc/not-doing.md` | This is the row that matters most, and §3.7 is the boundary that keeps it intact |
| **Architecture rule 4 — whatever a GUI can do, the CLI can do** | `CLAUDE.md` | Every part below is a CLI verb first. A shell surface is optional and later |

**The `not-doing` §1 row, quoted, because it is the one this could break:**

> **External data sources** — A live connection to a database or another document —
> `office:database-source`-style live queries — is out of scope; a spreadsheet that is a database
> client is a different program.

Nothing here reopens it, and the line is sharp enough to test: **a data source is never stored in
a document.** A path named on a command line at query time is an argument; a connection string
written into `content.xml` is a feature, and that feature is the one that is out. Opening a
document must never cause a database to be opened, because a document that reaches out when you
look at it is the same category of thing as a document that executes when you look at it — the
row above it in the same table.

Call it **R12**, and it is checkable the way R11 is: `grind-query` is not in the dependency graph
of `grind-sheet`, `grind-text` or `grind-core`, only of `grind-cli` and any shell that opts in.

---

# Part I — the query layer

## 3.1 The argument: grind is already an ingest path

This is the part that changes the shape of the project rather than adding a verb to it, and it is
not visible from inside a single document.

What is already true:

- **CSV and TSV import is built** — `sheet/src/csv.rs`, `grind sheet import-csv`, with the
  delimiter sniffed, the decoding tolerant, and the typing rule the *same* rule as typing into a
  cell (`App::enter_range`), so `007` stays text and an ISO date lands as a date in a column
  formatted for one.
- **`.xlsx` import is scheduled** — phase 11, `doc/xlsx-import.md`, one way in, its own filter,
  with a fidelity report.
- **"Format as table" is built** — `App::format_table`, `sheet/src/table_format.rs`: a rectangle
  with a header row, an autofilter over it and a named range around it, written as ODF's own
  `table:database-range`.

Read those three together and the claim is this: **after phase 11, `grind` can take almost
anything a third-party system will hand you and turn it into a document it fully understands.**
A payroll system exports CSV. An ERP exports `.xlsx`. A reporting tool exports both. Every one of
those is a *dump* — a flat rectangle with a header row, produced by a system that had a database
and gave you a picture of one.

And then the ingest path stops, one step before the step that was the point. You now have three
rectangles that came out of three databases, in one document, with no way to relate them except
formulas.

## 3.2 What a spreadsheet cannot do with three dumps

Say `hours.csv` (employee id, job id, date, hours), `jobs.xlsx` (job id, code, client, rate) and a
hand-kept `rates.fods` (employee id, name, cost rate). The question is the ordinary one: **cost
and margin per client per month.**

In a spreadsheet, with every function this build implements available:

- The join is `VLOOKUP` or `INDEX`/`MATCH`, **one column at a time**, one formula per cell. Three
  looked-up columns over 20 000 rows is 60 000 formulas, and `VLOOKUP` additionally requires the
  key to be the first column of its range, so half the work is rearranging other people's exports
  to suit it.
- The aggregation is `SUMIFS` over a helper column that concatenates the group key, because
  "group by" is not a spreadsheet concept — a pivot table is, and this build does not have one
  and `doc/not-doing.md` does not promise one.
- Every one of those formulas is **stored in the document**, walked by the recalc engine, and
  invalidated the next time someone re-exports the source with the rows in a different order.
- The result is a document whose real content is a join, expressed as sixty thousand copies of
  the same idea.

That is not a criticism of spreadsheets; it is what spreadsheets are. It is a precise statement of
what a relational engine is for, and of why "put the dumps in a spreadsheet" is where this
workflow currently goes to die.

With a query layer it is eight lines, evaluated once, and nothing is stored unless asked for:

```sh
grind sheet query ledger.fods \
  --attach jobs=jobs.xlsx --attach staff=rates.fods \
  --sql "
    select j.client, strftime('%Y-%m', h.date) as month,
           sum(h.hours)                as hours,
           sum(h.hours * s.cost_rate)  as cost,
           sum(h.hours * j.rate)       as billed
    from   hours h
    join   jobs.jobs  j on j.job_id = h.job_id
    join   staff.cost s on s.emp_id = h.emp_id
    group  by j.client, month
    order  by month, cost desc"
```

**The honest framing.** "Turn any CSV or Excel export into a relational data warehouse" is the
pitch, and the part of it that is literally true is the valuable part: it is a **join engine over
other people's exports**, with the import filters as the loading dock and no persistence of its
own. It is not a warehouse — there is no storage layer, no incremental load, no history, and
nothing runs when you are not looking. Most people asking for a warehouse want the join engine;
this document should not let the word do work the feature does not do.

## 3.3 Shape: two layers, and only one of them is clever

The materialiser walks the loaded `Document` and writes an in-memory SQLite database
(`:memory:`), runs the query, prints the result, drops the database. Nothing is written to disk,
nothing persists between invocations, and the same promise `grind lint` and `doc/view-modes.md`
both make holds here: **a derived view cannot go stale, and a stored one cannot not.**

**Layer 1 — the cell layer.** A relational restatement of the model, always present, identical
for every document:

```sql
sheets  (id, name, ordinal, visible)
cells   (sheet_id, row, col, addr, value_type,
         num, text, bool, formula, style_id, format_id)
names   (name, expression, scope)
deps    (from_sheet, from_addr, to_sheet, to_addr)
styles  (id, bold, italic, ink, ground, borders, …)
formats (id, kind, decimals, symbol, locale)
ranges  (name, sheet, first_row, first_col, last_row, last_col, kind)
```

`deps` is the one worth pausing on: it is `sheet/src/graph.rs`'s reference index — the same index
`view::Names` and the role overlay read, resolved through `Engine::area` so it cannot disagree
with the evaluator — exposed as rows. *"What feeds this total?"* becomes a recursive CTE rather
than a feature someone has to design a UI for, and *"what breaks if I delete this sheet?"* becomes
a join.

This layer answers questions **about the spreadsheet**. It is cheap, mechanical, and it is what
makes `grind sheet calculations`, `Ctrl+Shift+F` and half of `grind lint`'s rules look like
hardcoded instances of a general thing — which they are.

**Layer 2 — the table layer.** One SQL table per *tabular range*, with **column names from the
header row**, so `hours` above is a table and not a projection somebody wrote by hand.

This is the layer §3.1 is about, and the reason it is tractable is that **the document already
knows which rectangles are tables**. It is not a heuristic and it is not a new concept:

| Source of a table | Already built as |
|---|---|
| A range formatted as a table | `App::format_table` → ODF `table:database-range` with `table:display-filter-buttons` |
| An autofilter | `sheet/src/filter.rs`, the same element |
| A named range whose first row is text and whose body is not | `grind sheet name`, §5.11 |
| A whole sheet with a header row, when the sheet has nothing else on it | The import filters' own output shape — one CSV is one sheet |

The fourth row is what makes `import-csv` and the xlsx filter land somewhere useful without the
user doing anything: a freshly imported dump *is* a table, because that is all that is on the
sheet. The first row is what makes it *stay* right when the document grows around it — a table
someone drew a border on and put a chart beside is still `table:database-range`, and the
rectangle is still known exactly.

**Typing.** SQLite's dynamic typing is the right match and the reason this is not a swamp: a cell
is stored as its own SQLite type — `REAL` for a number, `TEXT` for text, `INTEGER` 0/1 for a
boolean, `NULL` for empty — and a column of mixed types is a column of mixed types rather than a
coercion decision taken on the user's behalf. Two rules are needed on top:

- **Dates are ISO text**, not the model's serial number, because `strftime` is the whole reason a
  person writes `--sql` against a date column. The serial is still reachable through the cell
  layer and through a named `num` column for anyone who wants it.
- **An error cell is `NULL`, and there is an `_err` companion column** naming the error. Silently
  turning `#DIV/0!` into `NULL` in an aggregate is how a wrong number gets into a report.

**Formulas.** A table-layer column holds the formula's *value*, which is what a question about the
data means. The formula itself is in the cell layer. A stale cached value is `grind lint`'s
problem and it already has a rule for it — which is an argument for running the linter first and
an argument against the query layer growing its own opinion.

## 3.4 Where the dumps are actually dirty

The claim "any CSV or Excel export becomes a table" is the part of this that would embarrass us
first, so it is worth writing down what real exports do:

| What the export does | The answer |
|---|---|
| A title row and a blank row above the header | `--header-row 3`, or `--range A3:H20000` — explicit, per attachment |
| Two header rows, or merged header cells | Not supported; the column names come from one row. `--range` past it and use the cell layer, or fix it in the document with the editing verbs that already exist |
| A totals row at the bottom | `--range` again, or filter it out in SQL. `format_table`'s own totals row is known and **excluded** from the table layer, because grind wrote it |
| Blank rows separating groups | Rows are rows; `where col is not null` |
| A column that is numbers with three text cells in it | Dynamic typing carries it. `typeof(col)` is available in the query, which is a better answer than a coercion rule |
| Duplicate or empty column headers | Suffixed (`amount_2`) and, if empty, positional (`col_4`). Deterministic, and reported by `--schema` |
| 400 000 rows | See §3.8 |

`--schema` printing the derived tables and column types before anyone writes a query is not a
nicety here; it is how the user finds out what the header-row rule decided.

## 3.5 Several documents: `--attach`

One document is the small case. The case that makes this a different kind of tool is three
exports from three systems that nobody will ever join for you, and **the import filters are what
make that possible without a loader**:

```
--attach name=path
```

Each attachment is a document `grind` can already read — `.fods`, `.ods`, `.grind`, `.csv`, and
after phase 11 `.xlsx`. It is read with the ordinary reader, materialised into the same in-memory
database under its own SQL schema name, and dropped when the process exits. No file is modified;
none is even opened for writing.

This is where the CSV/xlsx argument pays: the loading dock is not something this feature has to
build. It is the existing import path, and every format grind ever learns to read joins the query
layer on the day it lands. A tool that reads five formats and can join across them is a
meaningfully different tool from one that reads five formats.

**And the attachments are arguments, never document content.** That sentence is §2's boundary
restated, and it is the only thing standing between this and the `not-doing` row. A shell that
offers this must ask for the files each time or keep the list in *its own* state, never in the
document.

## 3.6 Landing the answer back in a document: `--into`

A query you can only look at is half a feature; the whole workflow is dumps in, one question,
a table of results that a chart can be drawn on and a person can read.

```sh
grind sheet query ledger.fods --sql "…" --into 'Summary!A1'
```

**The mechanism matters more than the flag.** The result rows do not travel back through SQLite
into the model. `query --into` produces exactly what `import-csv` produces — a rectangle of
strings — and writes it with `App::enter_range`, in one `Action::Batch`, with one inverse, one
Ctrl+Z, R6 splicing on the way out and loop C over the result. It is an *ordinary edit that a
query happened to compute*, which is the same relationship `grind build` has to a document: the
arrow points one way and the document is the output, never a view onto something else.

Which means the answer to "why is this not `office:database-source` with extra steps" is
structural rather than a promise: there is nowhere to put the query. It is not in the file, the
document does not know it was produced by one, and reopening the document runs nothing.

Whether the header row lands as a header, and whether the landed rectangle is
`format_table`'d automatically, are two small decisions for whoever builds it; the default
should be yes to both, because a result table that is immediately filterable is the point.

## 3.7 The boundaries, stated as rules

1. **Read-only.** `SQLITE_OPEN_READONLY` on nothing, because there is no file — but the authorizer
   rejects everything that is not `SELECT`, plus `ATTACH`, `PRAGMA` and any function with a side
   effect. Accepting `UPDATE` would make SQL a second write path into the model, with the undo,
   splice, round-trip and projection obligations that every other write path carries.
2. **No source is stored in a document.** §2. This is the R12 line.
3. **Nothing is evaluated when a document is opened.** Materialisation happens when the `query`
   verb runs, and only then.
4. **The query layer never disagrees with the document.** Every table it exposes is built from the
   machinery that already answers that question — `graph.rs` for `deps`, `filter.rs`/
   `table_format.rs` for the tabular ranges, `numfmt` for the date rendering. §8 makes that a test
   rather than an intention.
5. **Bounded.** A statement timeout and a row cap, both with a flag, both defaulted. Same
   reasoning as the generator sandbox: an operation a person is waiting on must have an end.

## 3.8 Scale, honestly

The column store is an `mdds`-shaped run-length structure and this project's target is documents a
person edits, not a hundred-million-row fact table. Materialisation is O(cells) with an insert per
non-empty cell, and a 400 000-row CSV will take a noticeable second or two before the query runs.

That is acceptable for a CLI verb and it is not acceptable as something a shell does on a
keystroke, which is a real constraint on the shell surface (Q5) rather than an argument against
the feature. Two mitigations exist if it matters: materialise only the table layer when the query
does not mention a cell-layer table, and keep a cached database keyed by content hash — the
second of which is §5's workspace index, and is a good reason to build them in that order.

## 3.9 The reverse direction: `export-sqlite`

The cheap sibling, and it is not the same feature. `grind sheet export-sqlite book.fods out.db`
writes the table layer to a real file, for someone who wants the rows outside the office suite
entirely. Same spirit as `export-csv`, same one-way arrow, no import counterpart, and it is not a
document format for exactly the reasons in §1. It is genuinely a two-afternoon job once the
materialiser exists, and it should not be built before the materialiser exists, because the
schema it would pin is the materialiser's.

---

# Part II — the history store

**What exists.** `grind --session <path>` carries an undo stack between processes: the CLI reads
the file, calls `App::restore_session`, and writes `app.session()` back as JSON
(`cli/src/main.rs`). Undo history lives in the session, never in the document.

**What SQLite offers here** is specifically what the appfileformat document argues — a
trigger-managed undo/redo stack that survives session boundaries, with atomic commits — and the
fit is good because this is **disposable working state that nobody diffs and nobody hand-edits.**
A crash halfway through writing a JSON session truncates it; a crash halfway through a transaction
leaves the previous state.

```sql
session (id, doc_path, doc_fingerprint, created_at, grind_version)
ops     (seq, session_id, kind, sheet, addr, before, after, at)
cursor  (session_id, position)
```

Undo is `position--` and replay of `before`; redo the inverse. Each invocation is one transaction.

**Two design points to fix before building, not after:**

- **`doc_fingerprint`** — a hash of the document as read. If the file changed underneath the
  session (someone opened it in the GNOME window, or edited the `.grind` in vim), the stack is
  invalid and the command must **refuse** rather than replay onto a different document. This is
  the failure mode the JSON store handles badly today and the strongest single reason to touch
  this at all. Note that it is worth fixing *with or without* SQLite.
- **Location.** Beside the document in a gitignored directory, or an XDG state dir. Beside is
  friendlier for the CI and agent uses this CLI is aimed at; state dir litters less. Probably
  beside, because a document under `grind build` is a build artifact and its session is too.

**Why it is the safest of the four**: deleting it loses only undo history, and ripping SQLite back
out costs a serializer. It is also the smallest win, which is why it is not first.

**What it does not do**: give `grind text` an undo stack. That is blocked on making
`grind_text::Action` serialisable — a decision about the model, recorded in `doc/not-doing.md`'s
word-processor table — and no storage choice unblocks it.

---

# Part III — the workspace index

One `.db` per repository indexing **many** documents, which is where relational earns its keep:
cross-document questions have no home in any single-document format, and no amount of work on one
document's form creates one.

```sql
documents     (path, kind, mtime, content_hash, indexed_at)
doc_sheets    (path, sheet)
doc_names     (path, name, expression)
doc_cells     (path, sheet, addr, formula, value_type)   -- formulas only
diagnostics   (path, rule, severity, at, message)        -- grind lint
external_refs (path, target_path, target_sheet, target_addr)
```

**What it enables:**

- **Editor and agent support.** "Find all references" to a rate-card cell, completion drawn from
  every name in the workspace, go-to-definition on a named range. `doc/editor-setup.md` is
  highlighting and snippets today; an index is the substrate for anything past that. An agent
  asking *"which documents in this repo read the 2026 rate card?"* currently greps KDL — which
  works on `.grind` and not on `.fods`, and not at all on `.ods`.
- **Incremental lint.** `content_hash` means CI re-lints what changed. On a repository with
  hundreds of documents that is the difference between a gate people keep and one they turn off.
- **A materialisation cache for Part I** (§3.8), keyed by the same hash.

**Critical property, and it is the same one as everywhere else here: fully derived, disposable,
gitignored.** `grind index --rebuild` reconstructs it from the documents; deleting it loses
nothing. It is a cache, and it must be said to be a cache loudly enough that nobody is ever
tempted to treat it as a source of truth — which is the failure mode of every index that was
allowed to hold one field nothing else held.

---

# Part IV — the generator's data source

**Today.** `grind build timesheet.rhai -o month.fods` reads input data as JSON from one named
directory — `json("prices.json")`, `doc/generator-spec.md` §3.5, with `..`, absolute paths and
symlinks out all refused and `build/tests/data.rs` testing each wall.

**The addition.** Let a script open a SQLite file **read-only** from that same directory and
`SELECT` from it.

```rust
let rows = db("hours.db").query("
  select e.name, j.code, sum(h.hours) as hours
  from hours h
  join jobs j      on j.id = h.job_id
  join employees e on e.id = h.employee_id
  where h.month = ?
  group by e.id, j.id", [month]);
```

**This extends the cage rather than opening it**, which is the only reason it is admissible: the
same one data directory, `SQLITE_OPEN_READONLY`, an authorizer rejecting everything but `SELECT`,
no `ATTACH`, no `PRAGMA`, no temp tables, and a statement timeout so that "everything is bounded"
stays true. Every wall `json()` has, plus two the file format needs.

**Why it is a natural source rather than a new concept.** JSON stops scaling at a few hundred rows
and has no joins, so relational input gets denormalised by hand into the script — which is the
thing `doc/generator-guide.md`'s timesheet example spends its length avoiding. The data already
lives in SQLite in the real world: time tracking, inventory, an ERP export, a `.db` a reporting
tool dumped.

**The boundary to hold** is the one `doc/dsl.md` §2 already holds and it is unchanged: still
one-way. A script produces a document and is never recovered from one. A SQLite data source must
never become a live binding where a document "reflects" a database, because then opening a
document means evaluating something — R11, and the top two rows of `doc/not-doing.md` §1.

**Its relationship to Part I** is worth noting and not building on: `query --into` and a generator
reading a database are two different tools for two different jobs — one question answered into an
existing document, versus a whole document generated from data. They should stay separate, and
neither should grow toward the other.

---

## 7. Milestones

Four independent tracks; none blocks another, and the order below is by ratio of capability to new
surface, not by dependency.

| | Milestone | Done when |
|---|---|---|
| **Q0** | The materialiser and the cell layer — `grind-query` crate, feature-gated, no CLI yet | Every R7 document materialises; `deps` agrees with `graph.rs` cell for cell |
| **Q1** | `grind sheet query --sql`, `--schema`, text/JSON/CSV output, authorizer, timeout, row cap | A query runs, and no statement that is not a `SELECT` does |
| **Q2** | The table layer — `table:database-range`, autofilters, named ranges, lone-sheet imports; `--header-row`, `--range` | A CSV imported with `import-csv` is queryable by its own column names with no flags |
| **Q3** | `--attach name=path`, over every readable form | The §3.2 worked example runs against a `.csv`, an `.fods` and (phase 11) an `.xlsx` |
| **Q4** | `--into`, as `enter_range` in one undo entry, optionally `format_table`'d | One Ctrl+Z; loop C green over the result; R6 splices it |
| **Q5** | A shell surface — Ctrl+K in `ui_web` and `ui_sheet_gtk`, `:query` in `grind-tui` | §3.8's cost is not paid on a keystroke |
| **Q6** | `export-sqlite` | Round-trips through `sqlite3` and nothing imports it back |
| **H0** | `doc_fingerprint` and refusal on mismatch — **in the JSON store**, no SQLite | A session replayed onto a changed document is an error, not corruption |
| **H1** | The SQLite session store behind the same `--session` flag | The JSON path still reads; migration is one-way and silent |
| **I0** | `grind index --rebuild`, `documents`/`doc_names`/`doc_cells` | An index over this repository's own `examples/` |
| **I1** | Incremental by `content_hash`, and `diagnostics` from `grind lint` | Re-linting an unchanged repository does no parsing |
| **I2** | The materialisation cache Q5 wants | §3.8 |
| **P0** | `db()` in the generator sandbox, with §6's five walls | `build/tests/data.rs`'s walls, restated for a database |

**H0 is the one to build regardless of whether any of this is ever scheduled.** It is a
correctness bug in something that already ships, it is a day's work, and it needs no dependency.

## 8. Verification

The query layer's central risk is not that it returns a wrong row; it is that it becomes a
**second opinion about the document** that drifts from the first. The check that matters is
therefore a differential against the machinery that already answers each question:

- **`deps` against `graph.rs`** — every edge, both directions, over loop A's corpus. The reference
  index is already resolved through `Engine::area`; a second walk that disagreed would be a bug in
  the materialiser by construction.
- **Every built verb restated as SQL** — `grind sheet calculations`, `filter`'s predicate,
  `view::Names`' substitution, each `grind lint` rule that is expressible — must give the same
  answer as the verb. Where it cannot, that is a named gap, listed, with a test.
- **Nothing is written.** The `doc/view-modes.md` headline check, borrowed verbatim: open every R7
  document, materialise it, run a query, save, assert the bytes are identical.
- **`--into` is an ordinary edit**, so it inherits loop C and loop F with no new machinery — which
  is the whole reason §3.6 routes through `enter_range` instead of anything cleverer.
- **The authorizer**, positively and negatively: a table of statements that must be refused
  (`UPDATE`, `ATTACH`, `PRAGMA`, `load_extension`, a write via CTE) with a test per row.
- **Loops A, B, C, E and F are untouched**, because no new form and no new write path exists. If a
  change here needs a change there, something in §1 has been broken.

For the other three: the session store's check is a fingerprint mismatch refusing; the index's is
that `--rebuild` from scratch and an incremental update produce the same database; the generator's
is `build/tests/data.rs`'s four walls plus the two new ones, and determinism — the same script and
the same `.db` produce the same bytes.

## 9. What this will not do

| Not doing | Because |
|---|---|
| **A fourth document form** | §1. Nothing below it is negotiable while the third form's pitch is diffability |
| **`UPDATE` through the query layer** | §3.7. A second write path carries every obligation the first one has and would be given none of them |
| **A stored connection, a refresh button, a query saved in a document** | `doc/not-doing.md` §1 "External data sources", and §2's R12. This is the row this whole document is closest to and it stays closed |
| **SQL as a formula language** | `=QUERY(…)` in a cell means evaluating SQL on open. R1 and the macro line, at once |
| **A pivot table** | Genuinely adjacent, genuinely a different feature — it is a UI and a layout model, not a query engine, and `doc/not-doing.md` does not promise one |
| **Writing `.db` as the session's public format** | The session file is an implementation detail and should stay one. Nobody should build a tool against its schema |
| **Bundling SQLite unconditionally** | R3 and the size budget. Feature-gated, its own crate, like `sheet-xlsx` |

## 10. Risks, honestly

- **The `not-doing` row is one flag away the whole time.** Every user who likes `--attach` will
  ask for the attachments to be remembered, and the obvious place to remember them is the
  document. The answer is no, and it will need to be given more than once — which is why §2 states
  it as a checkable rule rather than as a preference.
- **Two query vocabularies.** OpenFormula in cells, SQL beside them, with different semantics for
  nearly everything that matters — `NULL` versus an empty cell, text-to-number coercion, collation
  and case, date epochs, integer division. `doc/dsl.md` §7 already records the two vocabularies
  that disagree about a *single word*; this is that problem an order of magnitude larger. The
  mitigation is that SQL never touches a cell's value: it reads values out and hands rows back,
  and no coercion crosses the boundary in either direction unnamed.
- **The table layer's header-row rule is a heuristic with a good disguise.** `table:database-range`
  is exact; "a sheet with one rectangle on it" is not. §3.4 is the honest list and `--schema` is
  the escape hatch, but the first bug report will be a dump whose header grind read wrong.
- **Size and dependency.** `rusqlite` with bundled SQLite is roughly a megabyte on a 3.6 MB binary.
  Feature-gating keeps that off the default build and *on* the build most people download, which
  is the same trade phase 11 makes — worth stating in the release notes rather than discovering in
  the size table `artifacts.yml` prints.
- **Scope gravity.** A join engine attracts a scheduler, a materialised view, a saved query and a
  dashboard, in that order. The feature line is the product (`doc/not-doing.md`), and this is a
  verb, not a subsystem.
- **It might be the wrong shape entirely.** The honest alternative is that everything Part I does
  could be a *projection* question instead — the `.grind` form is already text, a query over it is
  a different kind of tool, and `grind test` (D8) is the unbuilt half of the language that was
  supposed to answer "ask a question about a document". If D8 lands first and answers most of
  §3.1, this document shrinks to Part II and Part III.
