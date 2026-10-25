<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# Formula assistant — what A1 and A2 taught, 2026-10-20/21

Working notes, deliberately **not committed**: everything learned while building and running the
local-model benchmark, so the next session starts where this one stopped. The design is
`doc/formula-assistant.md`; §5.1 there is the formal write-up of the same numbers.

## The answer first: which model did best

**None of them is good enough, and SmolLM3-3B is the only one that could be measured here.**

| Model | Best top-1 (first surviving candidate right) | Ceiling (any candidate right) | Runs on this laptop? |
|---|---|---|---|
| **SmolLM3-3B** (Q4_K_M, 1.9 GB) | **13/188 (7%)** — catalog, no fence, first of 5, Excel read (8/188 without) | 14/188 (7%) — catalog, fenced, any of 5 | Yes: ~3 GB, ~4 s a task to read, 5–9 s a candidate |
| **OLMo 3 7B Instruct** (Q4_K_M, 4.5 GB) | not measured | not measured | **No**: exhausted swap beside a desktop on 16 GB; 1.8 tokens/s; stopped |
| Apertus 8B | not tried | — | Would not fit either (bigger than OLMo) |

So the "best" is SmolLM3-3B by default, at a level that is a **no-go** for the milestone's exit
criterion. Its best configuration was the plainest: the catalog in the prompt, no fence, the
first of five candidates that parses and evaluates. Roughly 12% of tasks can be hit by chance
(17 expect a logical, 6 expect 0 or 1), so differences of a few tasks between columns are noise.

## Prompt variants, 2026-10-21 afternoon: they do not help

Asked: could a better system prompt make the model respond to grind's specifics? Tried on a fixed
40-task sample (`--sample 40`, every ~5th task, all ten documents), three candidates each,
SmolLM3-3B. Baseline is last night's run restricted to the same 40 tasks.

| 40 tasks | baseline | V1 rules | V2 rules + 12 examples | V3 = V2 with 12 relevant functions instead of the whole catalog | V4 = V2 + strict fence |
|---|---|---|---|---|---|
| greedy right | 2 | 1 | 1 | 1 | 1 |
| first of 3 right | 2 | 1 | 2 | 2 | 1 |
| any of 3 right | 3 | 1 | 4 | 3 | 3 |
| greedy unparseable / error / wrong | 20 / ? / ? | 17 / 15 / 7 | **3** / 24 / 12 | 5 / 22 / 12 | 16 / 12 / 11 |
| reply per candidate | 5.3 s | 6.6 s | 5.1 s | 3.4 s (but 19 s reading per task) | 12.9 s |

- **Rules** (name the sheet, `;` not `,`, `.` not `!`, prefer SUMIF/COUNTIF over VLOOKUP): no
  effect. The model writes `VLOOKUP(…, Sheet.Project!A2:B4, 2, TRUE)` right after reading them.
- **Worked examples** (12 request→formula pairs from *other* documents, in the cached system
  part): the one thing that changed behaviour — **unparseable answers 20 → 3**, better than the
  grammar ever managed (12) — and the model picks the right function family more often. But
  correctness did not move: it now writes the right function on the wrong cells (`MAX(C2:C11)`
  for `MAX(Marks.C2:E11)`, `ISTEXT(Samples.C10)` for `ISTEXT(Stock.C6)`): unqualified references
  that mean its own empty sheet, whatever the rules say.
- **Relevant functions only** (word overlap with name, alias and summary; ties to the most-used
  functions): no better, and slower — a per-request list cannot be cached (390 tokens, 19 s a task).
- **Strict fence** (V4: on an empty target sheet every reference must name another sheet,
  `Fence::for_context`): *worse in an instructive way*. Denied the `C2` it wanted, the model does
  not write `Marks.C2` — the grammar lets `C` continue only as a function name, so `C2` becomes
  `COLUMN(` and then `MAX(COLUMN(COLUMN(COLUMN(…` until the token limit. **A grammar removes what a
  model wants to say; it cannot supply what it should say instead**, and masking mid-word sends it
  somewhere stranger. Same for `SMALL(SMALL(SMALL(…` and `AVERAGE(AVERAGE(…`.

**Conclusion: SmolLM3-3B's limit is capacity, not prompting.** Examples teach it the spelling;
nothing taught it which cells a request is about. Results: `~/.local/share/grind/a1-results/prompt-variants/`.

Ideas the variants suggest, untried:
- **Choose, don't write.** Small models pick among options far better than they generate. Offer the
  glance's real ranges as whole alternatives (`Marks.C2:C11`, `Marks.E2:E11`, … one per column of
  each table) and let the model *select* a function and ranges — a classification task, with the
  grammar enumerating complete choices at the first token of a reference rather than masking a
  cell address mid-word.
- **A bigger model** where the memory exists (OLMo 3 7B setup works; needs the desktop closed or 32 GB).
- **A6**: fine-tune on synthetic, evaluator-labelled data — the examples result says the model *can*
  absorb grind's spelling; it needs the semantics too.

## A prompt for any model — built in every client (2026-10-21/22)

The direction the user chose after the local-model no-go: grind writes a prompt for **any** model
the person picks, and checks what comes back. Design: `doc/formula-assistant.md` §11 (decisions,
what was tried, open items). All committed and pushed on `feature/formula-assistant`.

- **Core** (`grind-assist`): `shape.rs` describes each sheet by its shape (header, data rows, per
  column header/type/hint, small tables shown whole); `copy.rs` has `copy_prompt`, `try_answer`
  (either syntax, filled and recalculated on a scratch copy, so running totals work and the
  document is untouched), `apply_answer` (one batch = one Ctrl+Z, via `shift::shift` +
  `enter_range` with the *stored* spelling) and `inherited_format` (format of what the value is
  carried from: SUMIFS's sum range, IF's branches, + and *; nothing for COUNT/MONTH/comparisons;
  never over an existing format; set *before* the formulas so one undo hides everything).
- **CLI**: `grind sheet prompt FILE RANGE "request" [--no-values]`, `grind sheet answer FILE RANGE
  'formula' [--apply]` (feature `assist`, default on; `examples/sample-sheet.sh` reaches both).
- **GNOME** (`ui_sheet_gtk/src/ask.rs`): palette *Ask Any Model for a Formula…* — request, Copy
  Prompt (shown too), answer, Check, Apply only for the checked answer. Screenshot-verified via
  Broadway.
- **TUI**: `:prompt [--no-values] words` (pane + OSC 52), `:answer f`, `:answer! f`.
- **Web**: Ctrl+K rows + `#ask` dialog with Apply (close listener reads `returnValue`);
  smoke-checked in jsdom.
- **Win32**: Data menu, three verbs over prompt/show_list/confirm (type-checked, not run in Wine).
- **Mac**: Edit menu, three verbs over NSAlerts; `asked.rs` portable+tested; AppKit type-checked.

Gotchas found on the way: `App::enter`/`enter_range` take the **stored** syntax (`[.A1]`), not
display; `!` outside a string must go to the Excel reading first (display reads it as
intersection → `#NULL!`); a pasted ISO date is only a date in a date-formatted cell; the used
extent counts formatted blanks; `pgrep -f`/`pkill -f` match the shell running them (use
`pgrep -x` or `ps | awk`); /tmp quota can fill (other sessions' scratchpads live there too).

Open: several tables on one sheet; a large label/value form; the Win32 and Mac halves have not
run; nothing measures a cloud model yet (the §5 harness could, with a backend for it — the user
would have to choose and run that, since grind has no network code).

## Reading Excel's syntax as an output filter, 2026-10-21 evening

Accept that the model writes Excel and translate it: `grind_assist::read` tries display syntax,
then `grind_xlsx::formula::translate` (the import filter's, a real parser to the core's tree). A
`!` outside a string goes to Excel first (display syntax reads it as intersection → `#NULL!`).
`--example rescore` re-checks saved runs offline. Best run (catalog, no fence, n=5): greedy 4 → 9,
first of 5 8 → **13/188 (7%)**, any 9 → 15, unparseable 117 → 25 — now SmolLM3's best result.
But only 5 of the 92 newly readable greedy answers were right: syntax hid the semantic failure.
Fenced runs unchanged (the fence already forced display syntax). Keep it: syntax solved cheaply,
no fence needed, and it helps a person who types Excel too.

## Every run (SmolLM3-3B, all 188 function-sample tasks)

| | catalog, no fence (n=5) | no catalog, no fence (n=5) | catalog, fence v1 (n=5) | catalog, fence v2 (n=3) |
|---|---|---|---|---|
| greedy right | 4 | 3 | 2 | 3 |
| first of 3 right | 7 | 3 | 5 | 6 |
| first of 5 right | 8 | 3 | 6 | — |
| any right | 9 (of 5) | 3 (of 5) | 14 (of 5) | 13 (of 3) |
| greedy misses: unparseable / error / wrong | 117 / 31 / 36 | 142 / 19 / 24 | 66 / 73 / 47 | 52 / 77 / 56 |
| prompt read per task; reply per candidate | 3.9 s; 5.3 s, 20 tok | 1.5 s; 5.0 s, 30 tok | 3.5 s; 30.7 s, 120 tok | 3.6 s; 9.4 s, 34 tok |
| wall time | 146 min | 136 min | 384 min | 175 min |

Fence v1 forbade the trailing newline (bug, below); v2 allows it. Raw replies, verdicts and
timings for every task: `~/.local/share/grind/a1-results/` (`a1/*.jsonl`, `a1-fix/*.jsonl`,
`table.md`, `progress.log`, `overnight.log`).

## How the model fails

1. **Excel, not OpenFormula.** Commas between arguments, `Sheet!A1`, whole-column references. It
   persists with the instruction *and* every signature in the prompt. The fence removes it.
2. **`VLOOKUP` for everything.** Database functions, `ROW`, `COUNTIF`, text functions — the
   model's prior is a lookup. Not fixable by syntax.
3. **The wrong sheet.** The formula sits on an empty `Samples` sheet; the data is on other sheets
   the glance shows. The model writes unqualified references (`D2:D9`), which mean the empty
   sheet. The single commonest wrong answer, and the cheapest thing to try next: say in the
   prompt which sheet is empty and that every reference should name its sheet.
4. **Headers as references.** `Shipping.From kg` — reads a column label as a name. Fence removes.
5. **Rambling and degeneration.** Without a stop it explains; with the fence it extends formula
   after formula (`& " " & …`), and even with fence v2 a quarter of greedy answers degenerate into
   repetition (`--VYR1:VYR9 --VYR2:VYR8 …`, `*60/60*60/60…`) until the 120-token cap. All 52
   unparseable fenced-v2 greedy answers are such truncations.
6. **The catalog barely helps.** No catalog is about as good at top-1 (3 vs 4). A 3B model does
   not use 4,500 tokens of signatures.

## What the setup taught (would bite again)

- **Order the glance so the varying part is last.** With the target cell first, every task
  re-read the whole document (535 tokens, 38 s). With it last, a task reads ~20 tokens (4 s) —
  llama-server keeps the longest common prefix. Same cost for the shipped assistant's second
  question in a document.
- **Prefill `=` and stop at `\n`.** Cut SmolLM3's explanations; works with `--jinja` and its
  template. Under OLMo's `--chat-template chatml` the prefill is *not* continued (the model
  answers `assistant`); `--no-prefill` exists for that, and OLMo answers cleanly without it.
- **A grammar must allow the end.** Fence v1 had no way to emit the newline the model wanted, so
  the stop never fired: 6.4 h instead of ~2 h. `root ::= expr "\n"?`.
- **The grammar costs generation speed**: ~3.6 tok/s fenced vs 5–7 unfenced on this CPU (a
  144-way function alternation in llama.cpp's GBNF sampler).
- **Context 8192, not 16384.** The catalog is ~4,550 tokens; the larger KV cache only made a
  16 GB machine swap.
- **OLMo 3's GGUF (lmstudio-community) needs `--no-jinja --chat-template chatml`**: its template
  uses a Jinja filter llama.cpp lacks and the server refuses to load. With `-c 6144 -fa on -ctk
  q8_0 -ctv q8_0` it loads, then swaps the machine to death beside a desktop.
- **This laptop** (i7-8565U, 4 cores, 16 GB, ~10 GB used by the desktop): prompt reading 18–19
  tok/s and generation 3–8 tok/s for 3B; 5.6 and 1.8 for 7B. 3B is the practical ceiling.
- **Shell gotchas:** `pkill -f <pattern>` kills the shell running it when the pattern is in its
  own command line (exit 144); `pgrep -f` in a wait loop matches the loop itself and never ends.
  Match with `ps -eo pid,args | grep "[p]attern"`. Never edit a bash script while it runs —
  bash reads it incrementally (the OLMo fetch died that way, after the download completed).
  Background shells get reaped under memory pressure; `nohup` runs survive.

## Reproduce

```sh
scripts/llama-server.sh serve                              # SmolLM3-3B, pinned, 127.0.0.1:8080
cargo build --release -p grind-assist --example a1
target/release/examples/a1 --catalog whole --fence off --candidates 5 --out DIR
target/release/examples/a1 --catalog whole --fence on  --candidates 3 --out DIR
GRIND_LLAMA_CTX=6144 GRIND_LLAMA_ARGS="-fa on -ctk q8_0 -ctv q8_0 --no-jinja --chat-template chatml" \
    scripts/llama-server.sh serve olmo3-7b                 # only on a machine with memory to spare
target/release/examples/a1 --no-prefill ...                # for OLMo
scripts/llama-server.sh stop
```

Models are in `~/.local/share/grind/models/` (both verified against `assist/models.tsv`).

## State of the work

- Branch `feature/formula-assistant`, pushed: `062a61b` (glance + `grind sheet assist`),
  `7ca5e76` (grind-assist, benchmark, llama-server setup), `ddc5e87` (the fence).
- **Uncommitted** in the tree (per the user's instruction on 2026-10-21): fence v2's newline and
  the strict empty-sheet rule (`assist/src/fence.rs`), `--no-prefill`, `--rules`, `--examples N`,
  `--catalog relevant:N`, `--sample N` (`assist/examples/a1.rs`), `Coaching`, `Catalog::Relevant`
  and the relevant-function ranking (`assist/src/lib.rs`), `Bench::sample`/`examples`/`pool`
  (`assist/src/bench.rs`), their tests, §5.1 of `doc/formula-assistant.md`, and this file. All
  tests and clippy pass.
- The server container is stopped; nothing runs.

## What to try next, cheapest first

0. *(Tried 2026-10-21, see "Prompt variants": rules, examples, relevant catalog and a strict fence
   do not move correctness. Items 1–3 below are done or refuted; 4–6 stand, plus "choose, don't
   write".)*
1. **Prompt: name the sheets.** Tell the model the target sheet is empty and every reference must
   be sheet-qualified; or fence it: allow an unqualified reference only when the target sheet has
   data. Directly attacks failure 3.
2. **Few-shot examples** from *other* documents' samples (never the task's own document), placed
   in the cached system part so they cost nothing per task.
3. **A repetition penalty / shorter `max_tokens` under the fence** (failure 5), and an arity-aware
   grammar (one comparison, argument counts per function) to cut the degenerate tails.
4. **A bigger model on a machine with memory**: OLMo 3 7B (setup above works), Apertus 8B (needs a
   pinned GGUF; only third-party quants exist). Measure on 32 GB, or with the desktop closed.
5. **A6 fine-tuning** on synthetic, evaluator-labelled data — the benchmark already scores it;
   keep the function samples out of training.
6. Separate or down-weight the 23 guessable tasks in the score.
