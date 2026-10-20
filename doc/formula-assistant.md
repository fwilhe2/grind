<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# The formula assistant — a local model that proposes, and grind that decides

**Status: a proposal, with A0 built and A1's setup built** (§4.2.1). No model is in the tree, and
nothing past A1 is decided until its numbers are in (§5.1). This document is the research behind "should grind use a language
model, and if so how", written so the decision can be made on evidence rather than on
enthusiasm. Its first milestone was a benchmark with no model in it at all: `App::assist_context`
(`sheet/src/assist_context.rs`), `grind sheet assist`, and the `grind-assist` crate (`assist/`) —
the prompt, the `Backend` trait, the check of one candidate, and the benchmark over the 188
function samples, proved by two stand-in backends in `assist/tests/bench.rs`.

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

### 4.3 The fence (A2)

`llguidance` (MIT, Rust): a Lark-like grammar compiled to a token mask per step, about 50 µs per
token. It is a crate, so the fence is the same whatever the runtime — llama.cpp embeds the same
library, and with candle it drives the sampling loop directly. The grammar is **generated** by
`grind-assist` from `funcs::implemented()` and the display lexer's token classes, never written
by hand.

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
| **Cloud backends** | Constraint 3. Not as an option either: an option is a default someone else picks |
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
