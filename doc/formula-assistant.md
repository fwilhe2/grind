<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# The formula assistant — a local model that proposes, and grind that decides

**Status: the local model is a no-go (§5.1); a prompt for any model is built (§11).** A0, A1 and A2
were built and measured: on the floor's hardware the best fully open model that fits, SmolLM3-3B,
gets 13 of 188 tasks right, and neither prompts, a grammar nor Excel's syntax read as a fallback
lift it much further. What *does* work is everything grind owns — describing a document, reading a
formula in either syntax, checking it — so §11 keeps those and lets the person choose the model:
`grind sheet prompt` writes a prompt to copy into any model, `grind sheet answer` checks what it
said and `--apply` writes it. The earlier sections are the record of how that was found.

## 1. What it is

A person describes a formula in words — *groceries paid by card*, *the month's total without the
hidden rows*, *Summe der Ausgaben im September* — and gets back one or more formulas in display
syntax, each shown with what it evaluates to at the cell it would sit in. Or a person points at a
formula and asks what it does, and gets a sentence whose every number came out of the trace.

Under four constraints, all of them the user's:

1. **Trustworthy.** A wrong answer must be caught before a person sees it, not after.
2. **Really open source.** Weights, training data and training code, under licences
   `core/tests/third_party.rs`' `ALLOWED` list would accept. Open *weights* alone do not qualify.
3. **Purely local.** Nothing leaves the machine. No account, no network, no telemetry.
4. **Small.** A laptop CPU with 8 GB of RAM and no GPU is the floor.

### The thesis

Under constraint 4 the model is the weakest part of the system, and the design should treat it
that way. A 3B model does not *know* OpenFormula: it will invent `SUMIFS` argument orders, call
functions grind does not have, and write Excel's `,` where ODF wants `;`. Grind already knows all
of that — as code, held to LibreOffice by six loops. So the model is not asked to know anything.
It is asked to **guess**, inside a fence grind builds, and grind **checks** every guess with the
machinery it already trusts. The domain knowledge is the catalog, the parser and the evaluator;
the model is a search heuristic over the space they define.

## 2. Decisions

1. **The model proposes; grind decides.** Every candidate is parsed (`display::parse_display`),
   evaluated at its cell (`App::preview`), traced (`App::trace`) and linted before anyone sees it.
   A candidate that fails any of them is dropped, not shown with a warning. Nothing is written but
   **Apply**, which is the formula workbench's decision 3 unchanged
   (`doc/formula-workbench.md`): the assistant is a way of filling the workbench's expression
   field, not a new way of editing a document.

2. **The knowledge is grind's, supplied at request time, never trained in.** The prompt carries
   the function catalog (`assist::function_lines` and `friendly::signature`, the same lines
   `grind sheet functions --long` prints) and the cell's surroundings. Because both are generated
   from the code, they cannot drift from it: a function added on Monday is offered on Monday, with
   no retraining. The catalog is 23,631 bytes today (measured, `functions --long | wc -c`), on the
   order of 6,000 tokens — small enough to send whole, which removes the retrieval step and the
   class of bug where the right function was not retrieved. Ranking a subset with
   `grind_core::search::score` is the fallback if A1 measures prompt processing on a CPU as too
   slow.

3. **What the model can say is fenced by a grammar.** Decoding is constrained to display syntax
   with function names enumerated from `funcs::implemented()`, so an unknown function, an
   unbalanced bracket or a `,` separator cannot be generated at all rather than generated and then
   rejected. The grammar is deliberately **looser than the parser** — it is a fence, not a second
   definition of the language, and decision 1 still parses everything — and it is held to the
   parser in one direction only: every formula in the function samples and in loop B's corpus that
   survives display form must be accepted by it (the grammar ⊇ what real documents say). A test
   that fails when a function is added and the grammar does not know it is free, since the name
   list is generated.

4. **Fully open models only, pinned by hash, never shipped.** A model qualifies when its weights,
   training data and training code are published under licences on `ALLOWED`. No model is
   compiled in, vendored, packaged or downloaded by grind: grind contains no network code today
   and this feature does not add the first. The user puts a model file where grind looks
   (`$XDG_DATA_HOME/grind/models/`, or `--model`), and grind checks its SHA-256 against a table of
   **measured** models — the ones A0's benchmark has a score for. An unmeasured file still runs,
   and every answer from it says *this model has not been measured*.

5. **Deterministic.** Greedy decoding, a fixed seed, a pinned model: the same document, cell,
   request and model file produce the same candidates, byte for byte. That is what makes the
   feature testable at all, and it is what a person needs to report a bad answer reproducibly.

6. **A crate of its own, optional, and no reader depends on it.** `grind-assist`, beside
   `grind-spell` and `grind-print`, behind an `assist` feature of `grind-cli` and each shell. The
   rule is R11's shape: no crate that opens a document may depend on it, checked against every
   manifest the way `build/tests/manifest.rs` checks `grind-build` (A3; at A0 the crate is only
   on that file's list of readers, so it may never link the generator either). **Off by default** until A3,
   unlike the other optional crates — an inference runtime is the largest dependency this
   workspace would take, and it has to earn the default with numbers.

7. **Explanation is deterministic first, and prose is checked.** `friendly::explain` and the trace
   already explain a formula without a model. The model may only *paraphrase* a trace, and its
   paraphrase is checked mechanically: every number in the prose must be a value the trace
   recorded or a literal in the formula, and every function it names must occur in the formula. A
   sentence that fails is dropped and the deterministic reading is shown instead. Free prose that
   cannot be checked is the one output this feature never produces.

8. **The person sees exactly what the model saw.** The prompt is built by one core function and
   printable from the CLI (`grind sheet assist`). Nothing about the document reaches the
   model that this command does not show, and nothing is logged to a file by default.

9. **Clean room is unchanged.** The model is never a source for a fact in `doc/ods-format.md`,
   `doc/xlsx-format.md` or any other spec, and is never asked to write grind's own code. Its
   training data almost certainly contains LibreOffice's source; that is irrelevant to a formula
   the evaluator then checks, and would not be irrelevant to anything else.

## 3. The models

Measured nothing yet; this is the field as of 2026-10-20. "Fully open" means weights, data and
code.

| Model | Params | ~Q4 file | Licence | Data | Why it is here |
|---|---|---|---|---|---|
| **SmolLM3-3B** (Hugging Face) | 3B | ~2 GB | Apache-2.0 | published | The size the floor wants. Tool calling, 128k context, six languages including German. **The default candidate** |
| **OLMo 3 7B Instruct** (Ai2) | 7B | ~4.5 GB | Apache-2.0 | Dolma 3, published | The most transparent family there is (checkpoints, OlmoTrace). The likely ceiling on CPU |
| **Apertus 8B** (Swiss AI) | 8B | ~5 GB | Apache-2.0 | published, opt-outs honoured | Strongly multilingual — requests in German are a real case here |
| SmolLM2-135M / 360M | <0.4B | <0.3 GB | Apache-2.0 | published | Too weak to judge; useful for **plumbing tests in CI**, where only determinism and the fence are asserted |

Excluded, and why, so the question is not reopened by accident: Llama and Gemma (licences with
use restrictions, not on `ALLOWED`); Qwen, Phi, Granite and Mistral's small models (open weights
under acceptable licences, but closed training data — constraint 2). Qwen in particular is likely
to *score* better on A0, and A1 may measure it as a reference point, so that the cost of
constraint 2 is a number rather than a guess. It is never in the measured table.

Ai2 published a new MoE training stack on 2026-10-01; a smaller OLMo may follow. The table is
re-read at A1.

## 4. The pieces

### 4.1 The context (A0, built, in `grind-sheet`)

```rust
pub fn assist_context(&self, sheet: usize, at: Pos) -> Result<AssistContext>
// AssistContext { at, locale, sheets: Vec<SheetGlance>, sheets_left_out, names }
// SheetGlance  { name, rows, cols, shown: Vec<Vec<Glimpse>>, rows_left_out, cols_left_out }
// Glimpse      { address, kind, input }
```

What a person would glance at before writing the formula: the target address, the document's
locale, the **top of every sheet** (40 rows, 26 columns), the 10 rows above `at` on its own sheet
when it lies further down, and the defined names in display syntax. Every cell is shown as
`App::input_text` puts it in the formula bar — a formula in display syntax, a date in ISO, a
number in the document's spelling — with text quoted, so `"3"` and `3` cannot be confused. At most
16 sheets (the target's always among them) and 80 characters a cell, and what is left out is
counted and said rather than silently cut, so a 500,000-cell sheet produces the same few
kilobytes a small one does.

The first draft of this section planned a per-column *summary* (header, kind, a handful of
values), guessed with `chart::guess`. Building it against the samples changed that: half the data
sheets are key–value forms (`Garden`: a label in A, a figure in B) rather than tables, and a model
writing `Garden.B11` needs to see the cell `B11` beside its label, which only the cells
themselves show. The cells, bounded, are both simpler and more honest than a guess about them.

`Display` is the text the model is given; `grind sheet assist book.fods C5` prints it, and
`--format json` returns the structure.

### 4.2 The backend (A1)

```rust
pub trait Backend {
    fn propose(&mut self, request: &Request) -> Result<Vec<String>, String>;
}
// Request { id, prompt: Prompt { system, user }, candidates }
```

One trait, so the benchmark can compare runtimes before one is chosen. The fence joins it at A2.
The candidates:

| Runtime | For | Against |
|---|---|---|
| **A local OpenAI-compatible server** (`llama-server`, over `127.0.0.1` only) | No dependency at all; any model; the fastest way to A1's numbers | Not "grind does it": the user runs a second program. Loopback HTTP is still network code |
| **`llama-cpp-2`** (bindings to llama.cpp) | The fastest CPU inference there is; **llguidance is built into llama.cpp** (b4613 onwards), so decision 3 is a string | A C++ build on four platforms, three of them cross-checked from Linux today (`doc/windows-shell.md`, `doc/macos-shell.md`) |
| **candle / mistral.rs** | Pure Rust, Apache/MIT; builds wherever the workspace builds | Slower on CPU in published comparisons; each model *architecture* must be supported — SmolLM3 and OLMo 3 need checking |

The plan is the first for A1 (it costs nothing and answers *is this worth doing*), and the choice
between the other two is A3's, on measured latency and binary size. The server backend may stay
afterwards as an option for someone who already runs one; it is never the default.

### 4.2.1 Reproducing A1 (built)

Two pins, both files in the tree, so a run is the same run on another machine:

- **The server**: `ci/llama-server-image`, llama.cpp's own server image by digest
  (`server-b11429`, published 2026-10-05).
- **The model**: `assist/models.tsv`, a URL fixed to a repository revision, the file's SHA-256,
  its size and its licence. The recommended model is **`smollm3-3b`**, `SmolLM3-Q4_K_M.gguf` from
  `ggml-org/SmolLM3-3B-GGUF` (llama.cpp's own conversion, 1.9 GB, Apache-2.0). `olmo3-7b`, the
  ceiling, is beside it.

```sh
scripts/llama-server.sh serve                        # fetch, verify, serve on 127.0.0.1:8080
cargo run --release -p grind-assist --example a1     # 188 tasks × 5, with and without the catalog
cargo run --release -p grind-assist --example a1 -- --catalog whole --limit 20 --out /tmp/a1
scripts/llama-server.sh serve olmo3-7b               # the next model, same port
scripts/llama-server.sh stop
```

The script is the one place a model is downloaded; it puts it in `$XDG_DATA_HOME/grind/models`,
refuses a file whose hash differs, and publishes the server on loopback only. `assist/src/server.rs`
is the backend: plain HTTP/1.1 over `std::net`, **refusing any address that is not this
machine's**, one request per candidate — candidate 0 greedy, candidate *i* sampled at 0.7 with
seed *i* — so the run is reproducible, and asking for five gives the one asked for one first.

What building it changed, each found by the first smoke run:

1. **The target cell moved to the end of the glance.** With it first, every task's prompt differed
   from its neighbour's in the first line, and the server re-read the whole document for each
   one: 535 tokens, 38 s a task on the floor's hardware. Last, the 18 to 23 tasks of one document
   share everything up to it, and a task costs about 20 tokens and 4 s. That is the shipped
   assistant's cost too — a second question in the same document — not a benchmark trick.
2. **The reply is prefilled with `=` and stopped at the line's end** (`Server::prefill`, on). Told
   to answer with the formula alone, SmolLM3 explained it anyway, at about 3 tokens a second:
   half a minute a candidate, and sometimes with the prose *before* the formula. The prefill is a
   one-character fence, and the table says it is on.
3. **The server's context is 8192 tokens, not 16384.** The catalog is about 4,550 tokens; a larger
   window bought nothing and cost a gigabyte of KV cache on a 16 GB machine that was already
   swapping.

### 4.3 The fence (A2, the grammar built)

`assist/src/fence.rs` builds the grammar **per request**: the operators and literals of display
syntax, the engine's own functions (`funcs::implemented()`), and the document's own sheet names
and defined names — so a function this build lacks, a sheet that does not exist, a name nobody
defined, Excel's `,` and `Sheet!A1`, a header used as a reference and a `...` are all unwritable,
not rejected afterwards. It starts after the `=` the backend prefills. Everything else is looser
than the parser on purpose: no arity, no types, and `check` still parses every candidate.

The grammar is data, rendered two ways: `Fence::gbnf`, the text llama.cpp's server takes in a
request's `grammar` field, and `Fence::accepts`, a small recogniser over the same rules. The test
that matters is the second against the first's input: **every one of the 188 reference formulas
is accepted by the grammar built for its own document**, so the fence can never be what caps a
model's score (`assist/tests/bench.rs`). A1's runner takes `--fence on|off|both`.

For measuring, the server's own GBNF engine is enough. For shipping, the plan is unchanged:
`llguidance` (MIT, Rust; a token mask per step in about 50 µs), which llama.cpp embeds and candle
can drive directly — the same rules, rendered to its Lark-like syntax at A3.

### 4.4 The loop (A2)

1. Build the context (4.1) and the prompt.
2. Ask for *n* candidates under the fence (default 5).
3. For each: parse, `preview` at `at`, `trace`, lint. Drop failures, drop duplicates by tree (not
   by text — `SUM(A1:A3)` and `SUM( A1:A3 )` are one candidate).
4. If the person gave an **expected answer** (`--expect 263.5`, or a cell in the pane), keep only
   the candidates whose value matches it. This is what turns the assistant from a guesser into
   something test-driven: a person who knows one right answer can stop trusting the model.
5. If the person gave **what-if pairs** (`--with`/`App::overlay`), every candidate must match on
   each of them too. One example is weak evidence; three are strong.
6. Rank what is left: fewest functions, shortest tree, no volatile calls — a plain rule, printed,
   never a model score.

Nothing survives → say so, and show the closest failure with why it failed. An empty answer is a
correct answer here; a confident wrong one is the only failure that matters.

## 5. The benchmark (A0)

The function samples are the benchmark, and they already exist: `sheet/tests/data/functions/`
has 188 calls, each with a realistic sheet, the formula, **a plain-English statement of what it
computes** (the README's *What it shows* column — *Groceries paid by card*, *Full boxes of
tiles*) and LibreOffice's result. Every row is a task: given the document, the cell and the
sentence, produce a formula.

Scored **by value, never by text**: a candidate is right when it evaluates to the oracle's result
at that cell, however it is spelled. `SUMIFS` and the `SUMPRODUCT` it replaced are both right.
Reported per model and per configuration:

| | |
|---|---|
| bare | no catalog, no fence — what the model knows on its own |
| + catalog | decision 2 |
| + fence | decision 3 |
| + check | decision 1, at *n* = 1, 3, 5 — top-1 accuracy after verification |
| + expect | step 4, with the oracle's value as the expectation — the ceiling |

plus the rates that say *why*: does not parse, unknown function, wrong separator, evaluates to an
error, evaluates to something else. And latency at the floor's hardware: time to first token with
the catalog in the prompt, and total time per request.

**The samples never become training data** (A6), or the benchmark measures memory. A second,
smaller set — the same tasks asked in German, written by hand — tests decision 2's claim that
multilingual requests need no special handling.

A0 builds the harness and runs it with a **backend that returns the reference formula**, which
proves the scoring is right before any model is involved, and one that returns garbage, which
proves the checks reject it. A1 adds models.

How A0 keeps the answers from the model: each task's document is the sample document with its
`Samples` sheet replaced by an empty one of the same name at the same index, so the target keeps
its address (which `ROW()` needs) and no other sample leaks through the glance. Every sample
names the data sheet it reads, so emptying the sheet it sits on changes no answer. Numbers are
scored to nine significant digits, not fifteen: the question is whether the model chose the right
formula, and the engine's last digits against the oracle's are `sheet/tests/function_samples.rs`'s
to hold — four of them differ there, on purpose (`DIVERGES`).

### 5.1 What A1 measured (2026-10-20/21)

One machine, the floor's: an Intel i7-8565U (four cores, a 15 W laptop part), 16 GB of RAM with
an ordinary desktop open beside the run, no GPU. The server and model pins of §4.2.1. Every run
is all 188 tasks; *greedy* is candidate 0, *first of n* is the first of n candidates that
survives decision 1's checks, *any of n* is §4.4's step 4 with the oracle's value as the
expectation.

| SmolLM3-3B (Q4_K_M) | catalog, no fence | no catalog, no fence | catalog, fence (first grammar) | catalog, fence (fixed grammar) |
|---|---|---|---|---|
| greedy, right | 4 (2%) | 3 (2%) | 2 (1%) | 3 (2%) |
| first of 3 right | 7 (4%) | 3 (2%) | 5 (3%) | 6 (3%) |
| first of 5 right | 8 (4%) | 3 (2%) | 6 (3%) | — |
| any of 3 / of 5 right | — / 9 (5%) | — / 3 (2%) | — / 14 (7%) | 13 (7%) / — |
| greedy misses: unparseable, error, wrong | 117, 31, 36 | 142, 19, 24 | 66, 73, 47 | 52, 77, 56 |
| per task: prompt read; per candidate: reply | 3.9 s; 5.3 s (20 tokens) | 1.5 s; 5.0 s (30) | 3.5 s; 30.7 s (120) | 3.6 s; 9.4 s (34) |
| wall time | 146 min | 136 min | 384 min | 175 min, 3 candidates |

**OLMo 3 7B was not measured**, and why is itself the answer for this hardware: loaded beside a
desktop on 16 GB — with a 6k window and a quantised attention cache — it exhausted swap (8 of 8
GB), read the catalog at 5.6 tokens a second (13 minutes for the first task) and generated at 1.8.
The run was stopped rather than let the out-of-memory killer choose a victim among the user's
applications. Two more facts it cost: the `lmstudio-community` GGUF's embedded chat template uses
a Jinja filter llama.cpp's engine lacks (the server refuses to load it; `--chat-template chatml`
is the workaround, since OLMo 3 speaks ChatML), and under that template a prefilled `=` is not
continued — the model answers `assistant` — so it would have run with `--no-prefill`, where it
does follow "the formula alone" (`=A1+A2+A3`).

What the numbers say:

1. **SmolLM3-3B is not useful for this, in any configuration measured.** The best top-1 figure is
   4%; the ceiling with the answer handed over is 7%. Decision 1 did its job — nothing wrong was
   *shown* as right — but a check that rejects almost everything is an assistant that answers
   almost nothing.
2. **The catalog is worth little and the fence is worth less than hoped.** Without the catalog the
   model is no worse at top-1. The fence removed Excel's syntax as intended (unparseable greedy
   answers 117 → 66) but the model's *choices* did not improve: it reaches for `VLOOKUP` for nearly
   every task, refers to the empty sheet the formula sits on instead of the data sheet the glance
   shows, and invents comparisons to fill a formula out. Syntax was the visible failure; it was
   not the main one.
3. **The first grammar had a bug that made it slower and worse**: it did not allow the newline a
   model ends a formula with, so the backend's stop at the end of the line could never fire and
   the model extended formula after formula with `& " " & …` until the 120-token limit cut it off
   unparseable — six hours for a two-hour run. Fixed (`root ::= expr "\n"?`), and the re-run is the
   last column: replies fell from 120 tokens to 34 and the run took less than half the time, but
   the score did not move. Every one of its 52 unparseable greedy answers still ran to the
   token limit — the model degenerates into repetition (`--VYR1:VYR9 --VYR2:VYR8 …`,
   `*60/60*60/60…`) rather than ending — which no grammar can fix.
4. **12% of the tasks can be hit by chance** — 17 expect a logical and 6 expect 0 or 1 — so a few
   points of any column here are not evidence of anything. A2's benchmark should weight or
   separate them.
5. **The floor's hardware is a hard constraint, not a footnote.** A 3B model costs about 3 GB and
   runs; a 7B model does not fit beside a desktop on 16 GB. The prompt cache (§4.2.1) is what
   makes even 3B usable: a first question in a document costs minutes, a second costs seconds.

**The go/no-go: no-go for SmolLM3-3B as the assistant's model.** The milestone's exit criterion
— *+ check* top-1 clearly useful — is not met. What would change the answer, cheapest first: a
prompt that names the data sheets and says that an unqualified reference means the empty sheet
the formula is on (the commonest wrong answer); few-shot examples drawn from the *other*
documents' samples; a bigger model on a machine that can hold one (OLMo 3 7B, measured on a
machine with the memory to spare); and A6's fine-tuning, whose synthetic data this benchmark can
already score. None of them is tried until the next one is chosen on purpose.

**Prompt variants, 2026-10-21** (40 evenly spread tasks, three candidates; `--rules`,
`--examples 12`, `--catalog relevant:12`, and a fence that requires a sheet name on an empty
target sheet): none moves correctness off the baseline's 2–4 of 40. Twelve worked examples from
other documents are the one change that alters behaviour — unparseable greedy answers 20 → 3 —
but the model then writes the right function on its own empty sheet's cells. The strict fence makes
it worse: denied `C2`, it continues `C` as `COLUMN(` and loops. A grammar removes what a model
wants to write; it cannot supply what it should. The limit is the model's capacity, not the
prompt. The next idea worth measuring is to make the model *choose* among the glance's real ranges
rather than write addresses.

**Reading Excel's syntax as well, 2026-10-21.** A candidate display syntax refuses is read
through the import filter's translator (`grind_xlsx::formula::translate`, `read`/`check_with`,
the runner's `--excel`): a parsed tree printed by the core's serialiser, never a rewritten string,
and Excel's `CEILING` arrives as `COM.MICROSOFT.CEILING`. A `!` outside a string sends a
candidate to Excel's reading first, since display syntax would take `Ledger!D2` for the
intersection operator. Re-scoring the saved runs with no model (`--example rescore`): the
catalog-and-no-fence run goes from 4 to **9** greedy, 8 to **13 (7%)** first of five and 9 to 15
any of five, with unparseable greedy answers 117 → 25 — the best result of any configuration, and
a cheaper fix for syntax than the fence. But of the 92 greedy answers it made readable only 5 were
right: Excel's spelling was hiding the real failure, not causing it.

## 6. The CLI (R9)

```sh
grind sheet assist  book.fods B9                                        # what the model would see; no model (A0, built)
grind sheet suggest book.fods B9 'groceries paid by card'               # candidates, each with its value
grind sheet suggest book.fods B9 'groceries paid by card' --expect -263.5 --with E3=Cash
grind sheet suggest book.fods B9 '…' --model ~/models/SmolLM3-3B-Q4_K_M.gguf --candidates 5
grind sheet explain book.fods B9 --prose                                # the checked paraphrase, or the reading
```

`suggest` writes nothing — applying a candidate is `grind sheet set`, as it is today. `--format
json` returns every candidate with its trace and, for the dropped ones, why. `doc/cli-parity-
sheet.md` gains the rows; `examples/sample-sheet.sh` gains no line, because the script must run
on a machine with no model, and that is named in the script rather than silently skipped.

## 7. The pane (A5)

No new surface. Under `doc/sheet-shell.md`'s *Four surfaces*, the assistant is the formula
workbench with one more field: **Ask**, above the expression. Its candidates appear as a list
under it, each with its value and the trace one click away; picking one loads it into the
expression, where every workbench feature applies to it like a formula the person typed; Apply is
still Apply. The *Inputs* column is where the expectation and the what-if pairs of §4.4 are typed,
so they are the overlay the workbench already has, not a second one.

A palette entry, *Ask for a Formula…*, opens the workbench with the field focused. No toolbar
button, no sparkle icon, no chat.

`grind-sheet-gtk` first, then the order `formula::assist` took. `grind-web` is a named gap: a
model in a browser tab is possible (WebGPU, wasm builds of llama.cpp) but a multi-gigabyte
download into a page contradicts decision 4.

## 8. Milestones

| | Builds | Done when |
|---|---|---|
| **A0** | `App::assist_context`, `grind sheet assist`, the benchmark harness over the function samples with two fake backends | **Done, 2026-10-20.** The reference backend scores 188/188; the garbage backend 0/188 with each of its three failures named on every task; no task's glance contains its own answer or its request; a 5,000-row sheet is bounded and says so. A whole prompt is about 17,700 characters, catalog included |
| **A1** | `Backend`, the loopback server backend, a first run of every model in §3 | A table in this document: each model × each configuration of §5, with latency on the floor's hardware. **The go/no-go**: if *+ check* top-1 is not clearly useful, stop here and say so |
| **A2** | The generated grammar, llguidance, §4.4's loop, `grind sheet suggest` | The grammar accepts every sample formula and loop B's corpus; no candidate shown to a person fails to parse or evaluate, asserted over the whole benchmark |
| **A3** | An embedded runtime, chosen on A1's numbers; the measured-model table; `grind-assist`'s manifest rule | Builds on all four platforms; CI's plumbing test with a 135M model is deterministic across runs; binary size and latency recorded |
| **A4** | `explain --prose` with decision 7's checks | No paraphrase that names a number outside the trace survives, asserted over every sample |
| **A5** | The workbench's Ask field in `grind-sheet-gtk` | `--render-to`, and the palette entry walked by `chrome_tests` |
| **A6** | *Only if A2's numbers say so*: a LoRA adapter trained on synthetic data grind generates and verifies itself | The benchmark improves on held-out samples; the dataset and the adapter are published under the same openness as the base model |

### On A6, if it comes

The training data would be generated, never collected: loop E's generator already produces
formulas from the catalog's signatures; a sheet and a sentence can be generated around each; the
evaluator labels every pair. Every pair is therefore correct by construction, the dataset can be
CC0, and the chain stays open end to end. **Not** LibreOffice's test documents and **not** the
Enron or EUSES corpora — their licences are unclear for training, and using them would blur the
line decision 9 draws. And not the function samples, which are the benchmark.

## 9. Not doing

| Not doing | Because |
|---|---|
| **A chat** | A conversation is unverifiable prose by construction. The assistant has two outputs — a formula and a checked sentence — and both are checked |
| **Writing without Apply** | Decision 1. Not even with a setting |
| **Cloud backends** | Constraint 3. Not as an option either: an option is a default someone else picks. §11 is not one: grind sends nothing, a *person* copies a prompt they can read into a model they chose |
| **Downloading models** | Decision 4. Grind has no network code |
| **The word processor** | Rewriting or generating a text document's prose is exactly the output decision 7 refuses: nothing can check it. A text-side feature would have to find its own checkable output first |
| **Generating macros, scripts or `grind build` generators** | A generator is code a person runs; code a model wrote and nobody read is the macro line from the other side (`doc/not-doing.md` §1) |

## 10. Open, and named

- **Is it worth it at all?** A1's go/no-go is a real question. A 3B model on a CPU may be too
  slow or too wrong even fenced and checked, and `friendly`, `assist` and the workbench may
  already be the better answer. Stopping after A1 is a successful outcome of this plan.
- **The prompt language.** The catalog is English; the request may not be. Decision 2 assumes the
  model bridges that. The German set in §5 measures it.
- **Locale in display syntax.** A German document's formula bar spells numbers its own way;
  the fence and the context have to agree with `reading.rs` about which.
- **Where the hash table lives.** In the source, as data like the dictionaries, updated when a
  model is measured — and held to `doc/third-party.md`'s rules once a model's licence is being
  *relied on*, even though the file is never shipped.
- **Hardware floor.** 8 GB and no GPU is a guess at what *small* means. A1 measures both the floor
  and an ordinary current laptop, and the floor may move.
- **GPU use.** Not ruled out — llama.cpp and candle both use Metal, Vulkan or CUDA when present —
  but nothing may *require* one, and every number in this document is a CPU number.

## 11. A prompt for any model (built)

The turn §5.1 points to. The benchmark found that the small local model is what fails and that
every part grind owns works: the glance, the reading of a formula in display syntax *or* Excel's
(`read`), and the check by evaluation. So grind keeps those parts and stops pretending to own the
model: it writes a prompt the person copies into whichever model they trust — a big local one, a
cloud one, a colleague — and checks what comes back before anything is written.

```sh
grind sheet prompt book.fods E2:E40 "sum all expenses of a month and show the result only in the first row of that month"
grind sheet prompt book.fods E2:E40 "…" --no-values     # headers and types only
grind sheet answer book.fods E2:E40 '=IF(…)'             # read, filled on a copy, every value shown
grind sheet answer book.fods E2:E40 '=IF(…)' --apply     # written: one batch, one undo step
```

### 11.1 Decisions

1. **Grind sends nothing.** There is no network code and this adds none: the prompt is text on
   stdout (in a window, on the clipboard). Constraint 3 is about what *grind* does; a person who
   pastes a prompt into a service has read it and chosen to.
2. **Describe the shape, not the cells** (`grind_assist::shape`). A table per sheet: the header
   row, the data rows, and per column its header, a type read from the values and their number
   formats — date, time, money with its symbol, percentage, number, text, TRUE/FALSE, mixed — and a
   hint: the distinct texts (four, then a count), the lowest and highest value, or a computed
   column's formula. A table ends at its last row with a value, since formatted blanks count as
   used. Compact enough to read before pasting it, which decision 1 depends on.
3. **`--no-values` keeps every value home.** Headers, types and the request; nothing a cell holds.
4. **Say where the formula goes and how it is filled.** A range means *the formula for its first
   cell, filled over the rest*: the prompt says so, says relative references move, and says `$`
   does not. Most requests that matter — a running balance, a per-row category, "the first row of
   each month" — are fill-down formulas.
5. **Either syntax back.** A capable model writes Excel by habit; the prompt says both are fine
   and `read` takes both (§5.1's last paragraph). The function list in the prompt is grind's 144,
   by name only.
6. **The answer is tried on a copy.** `try_answer` fills and recalculates a scratch copy of the
   document, so a formula reading its neighbours sees their new values and the document gets no
   edit, no undo entry and no notification. `--apply` (`apply_answer`) then writes the same
   formulas as **one** batch — one Ctrl+Z — moved to each cell as a fill moves them.
7. **A result takes the format of what its value comes from** (`inherited_format`): the amounts a
   `SUMIFS` adds rather than the dates it compares, both branches of an `IF`, the operands of `+`
   and `*` — and nothing for a count, a month, a comparison, a lookup or a function the table does
   not know. Only when every such cell shares one format, and only where the target has no format
   of its own. On apply the format goes in before the formulas, so one Ctrl+Z still takes away
   everything that shows.

### 11.2 Tried

A two-month ledger (Date, Category, Description, Amount in €), E2:E12, the request above. An
Excel-syntax answer of the kind a large model writes came back right: −1328.15 in September's
first row, −1340.35 in October's, −23.60 in November's, empty elsewhere. One try, not a benchmark —
§5's harness can measure a cloud model the same way when somebody chooses to.

### 11.3 Open, and named

- **The other windows.** `grind-sheet-gtk` has it (*Ask Any Model for a Formula…*, `ask.rs`: one
  dialog — request, Copy Prompt with the prompt shown, the answer, Check, Apply only for the answer
  checked). The terminal has `:prompt`, `:answer` and `:answer!` (the prompt in a pane and through OSC
  52). The browser has three Ctrl+K rows over an `#ask` dialog with Apply; Windows three Data-menu
  verbs over its existing prompt, list and confirmation modals. The Mac does not yet; it is the
  same two calls over `grind_assist::copy`.
- **Layouts that are not one table per sheet** — several tables on a sheet, a form of label and
  value — are described as one table with mixed columns.
- **Locale.** The prompt states the document's locale; numbers in the hints are shown as the cells
  show them, and an answer is read in the formula bar's own spelling or Excel's.

## Sources

- SmolLM3: Hugging Face, `HuggingFaceTB/SmolLM3-3B`, Apache-2.0, released 2025-07.
- OLMo 3: Ai2, released 2025-11-20, weights, Dolma 3 data, code and checkpoints under
  Apache-2.0; OLMo 3.1 32B 2025-12; OlmoCore 3, 2026-10-01.
- Apertus: Swiss AI Initiative (EPFL, ETH Zurich, CSCS), 8B and 70B, 2025-09.
- llguidance: `crates.io/crates/llguidance`, MIT; merged into llama.cpp as of b4613
  (`docs/llguidance.md` in that repository).
- Grammar-constrained decoding on small models: NVIDIA's technical blog, *Improving Bash
  Generation in Small Language Models with Grammar-Constrained Decoding* — 13 models, 62.5% →
  75.2% average pass rate with the fence, the closest published analogue to decision 3.
