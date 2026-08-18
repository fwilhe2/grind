# CLAUDE.md

Guidance for Claude Code in this repo. This file is kept short on purpose — the "why" behind
any given piece of code lives in that code's own doc comments and in `doc/*.md`; read those
when touching that area rather than expecting the full rationale here.

<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

## What this is

An ODF-native spreadsheet: one Rust core, native shells, a feature list that ends. Not a
port of LibreOffice, contains none of its code. `README.md` has the pitch, `CONTRIBUTING.md`
the contributor rules, `doc/plan.md` the phase plan and exit criteria.

`doc/plan.md`'s "The requirements" (R1–R7) is normative. In short: independence and
ODF-native semantics (R1); everything written validates against the RELAX NG schema (R2,
`jing -i`); minimal boilerplate (R3); `calcext:` opt-in, outranked by R2 (R4); LibreOffice's
files read, unknown properties inert (R5); writing changes as little XML as possible (R6,
phase 8, done); eight named documents that must load, vendored in `core/tests/data/kb/` (R7).
Strictness on the way out, tolerance on the way in — several R7 files are schema-invalid and
must still load.

## The two constraints that govern everything

**1. Clean room.** LibreOffice source may be *read* and *cited by `file:line`*, never
copied. Every fact learned from reading it goes into a cited spec document
(`doc/ods-format.md`) *before* it reaches code. LibreOffice is a conformance oracle and test
corpus, never a source — this is what keeps the project freely licensable.

**2. ODF semantics are the product.** No Excel-oriented libraries with a translation layer —
the divergence (error model, coercion, empty-cell handling, null-date, reference operators,
collation) is semantic, not syntactic, and a syntax translator leaks it. Normative sources:

| Source | Role |
|---|---|
| `doc/OpenDocument-v1.4-schema.rng` | ODF 1.4 Part 3 — content schema |
| `doc/OpenDocument-v1.4-os-part4-formula.html` | ODF 1.4 Part 4 — OpenFormula semantics |
| `doc/small-group.md` | The 110-function scope line, extracted from Part 4 §2.3.2 |
| `doc/ods-format.md` | Clean-room notes on undocumented LibreOffice behaviour |
| `doc/cli-parity.md` | Every public `App` method and the CLI command reaching it |
| `doc/gtk-shell.md` | Phase 9's GTK shell work plan — normative for that phase |
| `doc/not-doing.md` | The feature line as a product document |

Format-neutral plumbing (quick-xml, zip, petgraph, chrono) can be lazy; semantics never are.

## Commands

```sh
cargo test                       # everything
cargo test --test read_values    # one test file
cargo test -- repeated_columns   # one test by name substring
cargo clippy --workspace --all-targets   # must be clean; CI does not gate on it yet
reuse lint                       # must stay compliant; CI DOES gate on this
```

`sheet` is the CLI; every core capability is reachable from it, enforced by
`cli/tests/parity.rs`:

```sh
cargo run -p sheet-cli -- new book.ods
cargo run -p sheet-cli -- set book.ods A1 1
cargo run -p sheet-cli -- set book.ods A2 '=[.A1]*2'   # ODF syntax, verbatim
cargo run -p sheet-cli -- recalc book.ods
cargo run -p sheet-cli -- view book.ods A1:A2
cargo run -p sheet-cli -- --format json info book.ods
```

`sheet-gtk` needs `libgtk-4-dev` + `libadwaita-1-dev`, and is **not** in
`cargo build --workspace`'s path — built and run on its own:

```sh
cargo run -p sheet-gtk -- book.ods                  # .ods or .fods; no file = empty document
cargo run -p sheet-gtk -- book.fods --render-to /tmp/grid.png   # one frame, then exit
cargo test -p sheet-gtk                             # geom.rs, no display needed
```

`--render-to` is how a custom-drawn widget gets an assertable output (a refactor is proved
one when the PNG comes back byte-identical). Not a user feature.

```sh
cargo build && SHEET=target/debug/sheet examples/sample.sh /tmp/demo
cargo run -p sheet-gtk -- /tmp/demo/sample.fods
```

`examples/sample.sh` builds a document out of **every feature this build supports**, through
the CLI only, and `cli/tests/cli.rs` runs it — a feature without a line there is invisible.
Add one when adding a capability.

The corpus tests need a LibreOffice checkout and skip with a notice without one:

```sh
SHEET_LO_CORPUS=/path/to/libreoffice/core/sc/qa/unit/data cargo test
```

## The three loops

Correctness is checked against LibreOffice, not our own opinion. `soffice` must be on `PATH`
for loop C.

| Loop | Asserts | Where |
|---|---|---|
| **A** — read tolerance | every `.ods`/`.fods` loads without error | `core/tests/corpus_read.rs` |
| **B** — formula conformance | parse / display round-trip / evaluate-matches-cached-value | `core/tests/corpus_parse.rs`, `corpus_eval.rs` |
| **C** — round-trip differential | write → `soffice --convert-to` → read back → identical, and reverse | `core/tests/roundtrip.rs` |

`core/tests/kb.rs` is the fourth check and never skips: R7's vendored documents. It also
validates the writer against the schema (`jing -i`) and measures R3/R6.

Current scoreboard (see each test's own comments for what each column means and why):
loop A 358 read / 3 password-protected / 0 failed; loop B parse 75845/77061 (1216 named
syntactic exclusions); loop B display 75845 round-trip, 271 named ambiguity; loop B evaluate
13327/52213 matching LO (`FLOOR` in the test is the ratchet — raise it, never lower it; run
`SHEET_LOOP_B_DUMP=LOG cargo test --test corpus_eval -- --nocapture` for the scoreboard).
Loop C is green both directions and gates CI on the `out` direction.

Each loop has exactly one documented loosening (loop A accepts `Error::Encrypted`; loop C
compares doubles at 15 significant digits, all LibreOffice writes). A third exception is a
bug in the code, not the loop.

## Architecture

Shared Core / Native Shell (see
[fwilhe2/editor](https://github.com/fwilhe2/editor)'s `doc/shared-core-native-shell.md`).
All state and logic in `core/`; every shell is a renderer and event forwarder owning
nothing. `cli/` exists so capabilities cannot hide in a UI; `ui_gtk/` is held to the same
rule by `cli/tests/parity.rs`.

Rules that are cheap now, expensive to break later:

1. **Reads go through `App::get_viewport`.** No getter hands out the whole document.
2. **Undo/redo lives in the core.** `Document::apply` returns the action's inverse; that is
   the whole mechanism. Shells never implement history.
3. **The core pushes, shells never poll.** `App::mutate` drops the write lock *before*
   notifying observers, because an observer calls straight back in to re-read.
4. **Whatever any GUI can do, the CLI can do.** A UI-only feature is a bug.
5. **No filesystem assumptions.** Every `*_file` has a `*_bytes` twin.
6. **Every feature must survive a LibreOffice round-trip** (loop C).

### Where things live

- **`core/src/a1.rs`** — addressing. The *only* 0↔1 conversion in the workspace; a shell
  never does its own index arithmetic. Parses by wrapping in `[…]` and calling `lex::lex`.
- **`core/src/grid.rs`** — the column store: a run-length sequence of typed blocks
  (LibreOffice's `mdds` shape). Invariants restored by `normalize()`, asserted by `check()`.
- **`core/src/numfmt/`** — number formats (§5.2). Display only, never touches the value. No
  format-code strings (Excel's spelling, not ODF's) — a format is an ordered sequence of
  pieces. `preset`/`is_preset`/`preset_params` are the whole "set/read a format" vocabulary
  and live here so no shell invents a second one.
- **`core/src/locale.rs`** — decimal point and grouping separator only (two characters).
  Everything else (month names, CLDR tables) is a deliberate, named gap.
- **`core/src/style.rs`** — cell styling (borders, colour, alignment). ODF values kept
  verbatim; `PALETTE` (the clrs.cc colours) is the default a shell offers, never a limit.
- **`core/src/odf/`** — the reader. Tolerance is structural: an unrecognised element gets
  `Ignore` for its whole subtree, so unknown content is inert by construction rather than by
  special-casing. Dispatch is on `(namespace-uri, local-name)`, never prefixes.
  `core/src/odf/source.rs` is R6's diffable-write machinery: it retains the original bytes
  and splices edits into them instead of regenerating, falling back to a full regenerate
  whenever a change can't be spliced (format/style edits, a package, a repeated row, …).
  `core/src/odf/write.rs` is the regenerating writer, minimal by intent, pooling formats and
  cell styles so equal ones share one automatic style.
- **`core/src/formula/`** — the OpenFormula engine, built value model → lexer/parser/
  serializer → eval → functions, all cited to Part 4 by section. `value.rs` is the single
  point of failure for the value model, error set and §6.3 conversions. `eval.rs` recurses
  over the dependency graph rather than sorting one. `display.rs` is the formula bar's A1
  syntax layered on top of the same lexer/parser, not a second grammar. All 110 Small Group
  functions are implemented; `funcs::implemented()` is checked against `doc/small-group.md`
  by a test, which is the anti-bloat rule made mechanical.
- **`ui_gtk/`** — the GNOME shell (phase 9, `doc/gtk-shell.md` is normative here). Owns no
  data — every paint reads `App::get_viewport` and throws it away. Custom `gtk::Widget`
  drawing in `snapshot()`, not `GtkColumnView`. `geom.rs` holds all layout arithmetic and no
  GTK types, so it's unit-testable with no display. Every colour comes from the theme, never
  a literal, except the reference palette and a colour the document itself chose.
- **`cli/`** — `main.rs` (clap, one arm per subcommand) + `report.rs`. A subcommand is a few
  lines driving `App`; anything longer belongs in the core. `doc/cli-parity.md` +
  `cli/tests/parity.rs` are the parity ratchet: every public `App` method needs a reaching
  command or a named reason it has none, checked by a test that reads `core/src/lib.rs`.

## Conventions

- **Positions are 0-based in the core.** Only a UI is 1-based; the whole workspace converts
  in exactly one place — `core/src/a1.rs`.
- **`ponytail:` comments** mark deliberate shortcuts with a known ceiling and upgrade path.
  They're a tracked ledger — don't silently "fix" one without reading the reason.
- Deferred on purpose, documented where it lives: corrupt-zip recovery (no corpus file needs
  it; belongs with the spec's explicit repair mode).

## REUSE / licensing

**AGPL-3.0-or-later**, REUSE compliance enforced in CI. New files need annotating:

```sh
reuse annotate --copyright "Florian Wilhelm <fwilhelm.wgt+github@gmail.com>" \
               --license AGPL-3.0-or-later path/to/file
```

`Cargo.lock` is declared in `REUSE.toml` instead of annotated. **Never annotate
`doc/OpenDocument-v1.4-*` in place** — they're OASIS specs redistributed verbatim under terms
that forbid modification; they carry `.license` sidecar files instead. Anything *derived*
from them (`doc/small-group.md`) carries the OASIS copyright alongside ours.

## Where the work is

Phases 0–8 are done: harness, column store, undo/redo, reader (loop A green), writer (loop C
green both directions), the OpenFormula engine (all 110 Small Group functions, dates, named
expressions), number formats and cell styling, the CLI (with the parity ratchet), sheet
add/rename/delete, `doc/not-doing.md` (the product stop-line), and R6's diffable writer.

**Phase 9 (the shells) is done through M9**, planned in `doc/gtk-shell.md`. Done: M0–M8 — CI
split, the read-only grid, core prep (`a1`, `formula::display`, `enter`/`preview`/
`clear_range`/`enter_range`), selection/navigation, editing (in-cell editor + formula bar,
undo/redo, spoilage banner), clipboard, formula UX (point mode, autocomplete, signature
hints), styling + the format strip, and column widths/row heights (drag to resize,
double-click to autofit a column or clear a row, both surviving a LibreOffice round-trip).
What M8 does *not* do: grow a row to fit a wrapped cell or an oversized font — still clips,
tracked as a `ponytail:` note in `ui_gtk/src/grid.rs`.

**M9** brought packaging under `ui_gtk/data/` (`.desktop`, AppStream metainfo, a scalable
icon — nothing builds or installs them yet, since this is a pure Cargo workspace), a
`gtk::ShortcutsWindow` built from the same accelerator table the window wires up, recent
files via `gtk::RecentManager` (which `gtk::FileDialog`'s own "Recent" section already reads,
so no custom menu was needed), and the a11y floor — `gtk::Accessible::announce` on every
selection move, which is why `ui_gtk/Cargo.toml`'s `gtk4` feature is now `v4_14`. The flatpak
manifest was the one "stretch" item and was skipped.

The wasm shell after this is the honest test of rule 5 (no filesystem assumptions).
`doc/gtk-shell.md`'s "The gaps, written down" section is the up-to-date list of everything
deferred by decision in this phase.
