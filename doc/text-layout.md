<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Where layout lives — the decision `grind text` turns on

> **Status: DECIDED — Path C.** `doc/suite.md`'s fork was closed on Path A at S7, reopened
> immediately on the objection below, and settled on **Path C: line layout in the core, font
> metrics injected, pagination gated.** The five open questions are answered in "The decision"
> at the end, and those answers are as normative as the path itself. This document outranks
> `doc/suite.md`'s fork section, which is now the record of an argument rather than the answer.

This is the largest single decision left in the project. It decides what `grind text` *is*,
how much of it exists in `core/`, whether the terminal shell is possible, and roughly whether
the remaining work is months or years. It deserves its own document rather than a row in a
milestone table, which is what it had.

---

## The objection that reopened it

`CLAUDE.md`, "Architecture", first line:

> Shared Core / Native Shell. **All state and logic in `core/`; every shell is a renderer and
> event forwarder owning nothing.**

Path A puts line breaking in the shell. Three shells, three line breakers: Pango in GTK, the
browser in the web shell, character-cell counting in the terminal. That is not a renderer
detail. **It is the editing model**, and here is why:

| Operation | Defined in terms of | Under Path A, lives in |
|---|---|---|
| Down arrow, up arrow | *the next line* | the shell |
| Home, End | *this line's* start and end | the shell |
| Page Up / Page Down | a screenful of *lines* | the shell |
| Click at (x, y) → caret | hit-testing a *line box* | the shell |
| Shift+Down → selection | both of the above | the shell |
| Word wrap indicator, line count, "page 3 of 12" | lines | the shell |

A line is not a thing in the ODF document. It is an *output of layout*. So every caret
operation that mentions a line is downstream of layout, and if layout is in the shell then so
are they. Three shells would each implement Down-arrow, and they would disagree — not about
pixels, which nobody minds, but about **where the cursor goes**, which is the program's
behaviour. `doc/plan.md` rule 4 ("whatever any GUI can do, the CLI can do") has no answer at
all for Down-arrow under Path A, because the CLI has no width and therefore no lines.

That is the whole objection, and it is correct. S7's own recommendation section did not weigh
it — it argued from cost and from dependency count, and treated layout as rendering. It is not
rendering.

**What S7 got right and keeps.** The four caret *edits* — `insert_text`, `erase`,
`split_block`, `join_block` — are logical operations on the block sequence and involve no
layout at all. They are correct under every path below and none of that work is at risk. What
S7 got wrong is thinking they were the whole of the caret. They are the half that does not
need a line.

---

## The question the fork conflated

`doc/suite.md` offered two paths, and each answered **two** questions with one answer:

1. **Does the core own line layout?** — break opportunities, line boxes, caret movement by
   line, hit-testing, selection extents.
2. **Does the core own pagination?** — page boxes, `fo:break-before`, widow and orphan
   control, headers and footers, footnote placement, table splitting.

Path A said no to both. Path B said yes to both. **They are separable, and the objection above
is entirely about (1).** Nothing about the shared-core thesis dies for want of page boxes. It
does die if Down-arrow means three different things.

Seeing them as one question is what made the fork look like a choice between "a rich-text
editor" and "the thirty-years-of-edge-cases problem". It is not. There is a middle, and it is
where the argument actually points.

---

## The three paths

### Path A — no layout in the core *(chosen at S7, now in doubt)*

The core carries text; each shell wraps it however its toolkit does.

- **Core deps added:** none.
- **Core API added:** none.
- **Each shell implements:** wrapping, caret-by-line, hit-testing, selection extents.
- **Pagination:** never, without reopening.

### Path B — the full page model in the core

Line breaking, font metrics, shaping, tab stops, widows and orphans, footnote placement, table
splitting — all in `grind-core`/`grind-text`, with a shaping and metrics stack underneath
(`cosmic-text`, or `rustybuzz` + `fontdb` + `unicode-linebreak`).

- **Core deps added:** a font stack. The core stops being format-neutral plumbing.
- **Each shell implements:** drawing the boxes the core hands it. Nothing else.
- **Pagination:** yes, and it is the exit criterion.

### Path C — **line layout in the core, metrics injected, pagination gated** *(new)*

The core owns everything about layout that is **not** a font question, and asks the shell the
one question it cannot answer itself: *how wide is this text?*

```rust
/// How wide is a piece of text, and how tall is a line of it? The two things the core
/// cannot know, and the only two it has to ask for.
///
/// The unit is **the caller's own** and the core never converts: the terminal answers in
/// cells, GTK in Pango units, the browser in CSS pixels. The core does arithmetic against a
/// `width` supplied by the same shell in the same unit, and never invents one.
pub trait Metrics {
    /// The cumulative advance after each character of `text` — one value per character,
    /// the last being the whole string's width.
    fn advances(&self, text: &str, style: &TextStyle, out: &mut Vec<f32>);
    fn line_height(&self, style: &TextStyle) -> f32;
}
```

**Cumulative advances rather than a single width, and it matters.** An earlier sketch here had
`advance(&str) -> f32`, which forces the core to measure prefix after prefix to find a break
and re-measure to place a caret. Asking for the whole array in one call is what a shaping
engine naturally produces — Pango, the browser and a fixed-width terminal all answer it in one
pass — it is correct across kerning and ligatures because the provider sees the entire string,
and it means **everything after wrapping is metric-free**: the `Layout` carries the x of every
caret position, so hit-testing, `x_at` and caret movement are array lookups with no font in
sight. That is what makes a `Layout` a plain value a shell can hold, and it is why cost 1 below
is a solved problem rather than a mitigation.

The core then owns, once, for every shell:

- **Break opportunities** — UAX #14, via `unicode-linebreak`. A Unicode algorithm, not a font
  question, and neutral plumbing by this project's own ladder.
- **The line-breaking algorithm** — greedy line filling over those opportunities.
- **The `Layout` type** — lines, each a range of (block, run, character offset), with a height.
- **Caret movement by line, hit-testing, selection extents** — the six rows of the table above,
  in the core, identical in every shell, and reachable from the CLI because the CLI can supply
  a width and a fixed-width `Metrics`.

What stays in the shell: rasterising glyphs, and answering `advance`. That is a renderer's job
and nothing else is.

| | A | B | C |
|---|---|---|---|
| Line breaking | shell ×3 | core | **core** |
| Caret by line, hit-test, selection | shell ×3 | core | **core** |
| Font metrics / shaping | shell ×3 | core | **shell, behind a trait** |
| Page boxes, footnotes, widows | never | core | **gated, later** |
| Core dependencies added | none | a font stack | one Unicode table crate |
| Terminal shell possible | yes | only if parameterised | **yes, by construction** |
| CLI can answer "Down arrow" | **no** | yes | **yes** |

---

## What Path C costs, honestly

Four real costs, none of which the "one small crate" framing above should be allowed to hide.

### 1. An advance is not additive — solved by the trait's shape

`advance("a") + advance("b") != advance("ab")` — kerning and ligatures see to that. Summing
per-character widths is therefore wrong, and measuring prefix after prefix to avoid summing is
slow.

`advances()` above dissolves both: **one call per fragment**, the provider sees the whole
string and so applies its own kerning and shaping, and the core receives the cumulative array
it needs for wrapping *and* for caret placement. The residual inaccuracy is kerning **across a
fragment boundary**, which is a boundary between two different character styles — where
kerning is arguably wrong anyway. Named, and accepted.

### 2. Bidirectional text

ODF carries `style:writing-mode` (rng:2864) and real documents are RTL. Under Path A this is
free — Pango and the browser do it. Under Path C the core needs the UAX #9 bidi algorithm
(`unicode-bidi`) and, worse, has to decide what caret movement *means* in mixed-direction text:
logical or visual, and what Home does on a line that runs both ways.

This is the single largest hidden cost in Path C and it should be sized before committing, not
after. Two honest options:

- Take `unicode-bidi` and do it properly. It is the same class of dependency as
  `unicode-linebreak`.
- **LTR-only, with a named gate in `doc/not-doing.md` §2**, and a reader that still *preserves*
  `style:writing-mode` through R6. Refusing to lay out RTL is not the same as destroying it.

The second is defensible and is probably right for a first cut, but it must be a written
decision rather than an omission discovered by an RTL user.

### 3. Loop D needs a reference metric provider

If metrics come from the shell, then "our line breaks" is not one answer — it is one per shell.
A differential against LibreOffice therefore needs a *reference* provider, and a reference
provider means fonts, which means the font stack Path C was supposed to avoid.

Three ways out, in order of preference:

- **Do not make loop D Path C's exit criterion.** Test the *algorithm* against a synthetic
  provider where every character is one unit wide, which makes line breaking exactly assertable
  with no fonts anywhere. Then assert that the TUI and GTK shells, given the same document and
  the same width, produce the same *line structure* — same break points, in run/offset terms.
  That is the property the objection actually asks for: the shells agree about behaviour. It is
  testable today and needs nothing new.
- Keep loop D, and scope it to **pagination**, where fidelity against LibreOffice genuinely
  matters and where the pinned container's fonts pin the comparison.
- A dev-dependency-only reference provider (`fontdb` + `rustybuzz`) used by one test. Honest,
  but it is Path B's stack arriving through the back door and should be a conscious choice.

The first is the recommendation. **The exit criterion for core line layout is agreement between
shells, not agreement with LibreOffice** — the second is a pagination question.

### 4. It is a real milestone, not a refactor

Rough shape, in the plan's idiom:

| | What | Size |
|---|---|---|
| **L1** | `Metrics`, `unicode-linebreak`, the greedy breaker, the `Layout` type. Synthetic-provider tests | medium |
| **L2** | Caret by line, hit-testing, selection extents, on `App` and on the CLI (`--width`) | medium |
| **L3** | S8's terminal shell drives it with cell widths — the proof that injection works | (S8) |
| **L4** | S9's GTK shell drives it with Pango | (S9) |
| **L5** | S10's web shell drives it | (S10) |
| **L6** | Pagination, behind its own gate and loop D | very large |

L1+L2 sit **before** S8 rather than after it, which is a reordering of `doc/suite.md`'s
milestone list and the main scheduling consequence of choosing C.

---

## The case for Path A, stated fairly

It should not be dismissed, and it has three real points:

- **Pango and the browser are better at this than we will be.** Decades of bidi, shaping,
  hyphenation, locale-specific breaking and script support. Path C's core breaker will be
  worse, and visibly so for anything but Latin text.
- **A document looks native in each shell**, which is the *other* half of Shared Core / Native
  Shell and is not nothing.
- **The core stays dependency-free.** Two Unicode table crates is not much, but it is the
  first crack in "format-neutral plumbing only".

And the counter, which is why it still loses: those three are arguments about **rendering
quality**, and the objection is about **behaviour**. Shells disagreeing about glyph positions
is fine. Shells disagreeing about where Down-arrow puts the cursor is the thing this
architecture exists to prevent.

Path C concedes the first three almost entirely — shaping stays in Pango, in the browser, in
whatever the shell has — while taking back the decisions that are the program's own.

---

## What survives whichever way this goes

Nothing built so far is at risk, which is worth stating because it means this decision is not
urgent and should not be rushed:

- The model, `loc`, the reader, the writer, R6 splicing, loops A and C — all layout-free.
- S7's four caret **edits** — logical operations on blocks, correct under every path.
- The offset axis on `Loc` (`#intro+5`) — an addressing decision, not a layout one.

What is provisional is only the *documentation* of the decision: `doc/suite.md`'s fork section
and S7 row, `doc/not-doing.md`'s pagination row, and `README.md`'s paragraph. Those change with
the answer and nothing else does.

---

## The recommendation

**Path C.** The core owns line layout and every caret operation defined in terms of a line;
the shell answers `advance` and `line_height` and draws; pagination keeps its named gate and
its loop D, exactly as `doc/not-doing.md` §2 already has it.

It is the reading that takes the objection seriously — the logic that makes a word processor a
word processor ends up in the core, once, identical in every UI — without taking on the font
stack, and while keeping R10's terminal shell possible by construction rather than by luck.
`doc/suite.md` had already noticed that the terminal forces the engine to be parameterised by a
metric provider; Path C is that observation promoted from a footnote about Path B into the
design.

**The case against it**, so that choosing it is a decision rather than a drift: it is a
medium-plus milestone before any text shell exists, its line breaking will be visibly worse
than Pango's for non-Latin scripts, and the bidi question below has to be answered first rather
than discovered.

*(Chosen. See "The decision" at the end — bidi is answered by excluding RTL explicitly, which
is the trade that makes the second objection survivable.)*

---

## The decision

**Path C**, with the five open questions answered as follows. These are as normative as the
path; each is a boundary, and changing one is a product decision rather than a ticket.

### 1. Bidi — **out, explicitly, with a gate**

Layout is **left-to-right only.** `unicode-bidi` is not taken, UAX #9 is not implemented, and a
document containing right-to-left text lays out as though it were LTR — which is *wrong for
that document*, not merely unstyled.

This is a deliberate Pareto call, and the reason it is affordable is R6: **an RTL document is
still read, preserved byte-for-byte, and written back correctly.** Only the *view* is wrong,
and only for documents this build was never going to render well anyway. Refusing to lay out
RTL is not the same as destroying it, which is the same distinction `doc/not-doing.md` already
draws for change tracking.

`style:writing-mode` (rng:2864) is preserved and never consulted. The gate for reopening: a
real RTL document somebody wants to edit, at which point `unicode-bidi` goes in at L1's seam
and the caret-movement question — logical or visual — gets answered then rather than guessed
now. Recorded as a row in `doc/not-doing.md` §2.

### 2. Licenses — **checked, not assumed**

`unicode-linebreak` **0.1.5 is Apache-2.0** (verified in the crate's own `Cargo.toml`, not
inferred), implements UAX #14 against Unicode 15.0, and is compatible with AGPL-3.0-or-later.
It is a pure table crate: no font, no I/O, no unsafe dependency tree. That is one dependency
added to `grind-core`, and it is neutral plumbing by this project's own ladder — a Unicode
algorithm, not ODF semantics.

`doc/suite.md`'s claim that the *font* stack (`rustybuzz`, `fontdb`, `cosmic-text`) is
MIT/Apache remains unverified, and stays unverified: Path C does not take it.

### 3. `Layout` lives in **`grind-core`**, and the spreadsheet uses it

R8 is satisfied because line breaking over styled text mentions no document type's vocabulary:
its input is a flat sequence of `(text, TextStyle)` fragments, which a paragraph's runs and a
wrapped cell's display text both produce. `core/src/layout.rs`.

**And the spreadsheet adopts it.** `ui_gtk` M10's row auto-height already measures wrapped
text, so today there is a line breaker in the GTK shell that the word processor was about to
duplicate. One engine, two applications — which is an argument for Path C this document did not
count when recommending it, and the strongest available evidence that the abstraction is real
rather than invented for one caller.

### 4. The CLI measures **one unit per character**

`grind text view --width 72` uses a fixed-width `Metrics`, which makes every line operation
answerable from the CLI and rule 4 satisfiable for the first time. The core ships that provider
(`layout::Fixed`) because it is also what every test uses: a synthetic provider where each
character is one unit wide makes line breaking exactly assertable with no font anywhere.

A terminal shell wants `unicode-width` rather than a naive count, for CJK and combining marks.
That belongs in the shell, which implements the trait itself — the core stays free of it.

### 5. `ui_gtk`'s wrap measurement **moves onto the trait**

The GTK shell implements `Metrics` over Pango once, and both its grid and (at S9) its text view
use it. This is what makes question 3's answer load-bearing rather than aspirational, and it is
the migration that proves injection works against a real shaping engine.

### What this changes in the plan

`doc/suite.md`'s milestone list gains L1–L2 **before** S8, because the terminal shell is only a
pure renderer if the engine it renders exists first. That reordering is the main scheduling
consequence, and it is the point: S8 was going to discover this.

### What stays gated, unchanged

Pagination. `doc/not-doing.md` §2, loop D at a stated floor. Path C is line layout and nothing
more — page boxes, widows, orphans, headers, footers and footnote placement are all still out,
and the exit criterion for *this* work is agreement between shells, not agreement with
LibreOffice.

---

## Sources

| | Where |
|---|---|
| The fork as originally posed | `doc/suite.md`, "The layout fork" |
| The architecture rule the objection rests on | `CLAUDE.md`, "Architecture"; [fwilhe2/editor](https://github.com/fwilhe2/editor)'s `doc/shared-core-native-shell.md` |
| Pagination's named gate | `doc/not-doing.md` §2 |
| The caret edits that survive every path | `text/src/lib.rs`, `doc/cli-parity-text.md` |
| `style:writing-mode` | rng:2864 |
| `style:page-layout` / `style:page-layout-properties` | rng:12213, rng:12248 |
| `fo:page-width`, `fo:page-height` | rng:12255, rng:12260 |
| `fo:widows`, `fo:orphans`, `fo:keep-with-next` | rng:12512, rng:12517, rng:2110 |
| `style:master-page`, `office:master-styles`, `style:master-page-name` | rng:12141, rng:7939, rng:12803 |
| `style:header`, `style:footer` | rng:11962, rng:10844 |
| `text:note`, `text:notes-configuration`, `style:footnote-sep` | rng:8465, rng:17725, rng:10871 |
