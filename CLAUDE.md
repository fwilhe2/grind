# CLAUDE.md

Guidance for Claude Code in this repo. Keep it short: the reasoning behind a piece of code lives
in that code and in `doc/*.md`, and the history lives in git. Read those when touching an area.

<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

## What this is

An ODF-native office **suite**: one shared core, one crate per document type, native shells, a
feature list that ends. Not a port of LibreOffice and contains none of its code. `README.md` has
the pitch, `CONTRIBUTING.md` the contributor rules, `doc/plan.md` phases 0–9 and `doc/suite.md`
phase 10.

- **`grind sheet`** — the spreadsheet, complete through phase 9.
- **`grind text`** — the word processor: reads, writes, edits by block and by caret, lays text
  out, tables, Markdown in and out, PDF export. Not built: a session (no `undo` across CLI
  invocations), table styles and `table:formula`, footnotes, fields, style definitions, RTL.
- **Import filters**, one way in and never out: `.xlsx` (`grind-xlsx`), `.docx` (`grind-docx`).
- **Shells**: GTK (one per document type), terminal, browser, Windows, macOS (built, not yet run
  on a Mac). `doc/feature-matrix.md` is which client can do what.

`doc/plan.md`'s "The requirements" (R1–R7) is normative: independence and ODF-native semantics
(R1); everything written validates against the RELAX NG schema (R2, `jing -i`); minimal
boilerplate (R3); `calcext:` opt-in, outranked by R2 (R4); LibreOffice's files read, unknown
properties inert (R5); writing changes as little XML as possible (R6); eight named documents
that must load, vendored in `sheet/tests/data/kb/` (R7). `doc/suite.md` adds R8–R10, `doc/dsl.md`
R11. Strictness on the way out, tolerance on the way in: several R7 files are schema-invalid and
must still load.

## The README is the author's

**Never write `README.md` or `README.fodt`** — no edit, regeneration, export, restore, move or
delete, however small or obviously right the change. The author writes it by hand in
`grind-text-gtk` (`README.fodt`, exported to `README.md` with Ctrl+Alt+S). When something in it is
wrong or out of date, **say so in your reply**, quoting the line, and leave the fix to the author.
`.claude/hooks/protect-readme.sh` and the deny rules in `.claude/settings.json` enforce this; do
not work around or edit either.

## The two constraints that govern everything

**1. Clean room.** LibreOffice source may be *read* and *cited by `file:line`*, never copied.
Every fact learned from reading it goes into a cited spec document (`doc/ods-format.md`,
`doc/odt-format.md`, …) *before* it reaches code. LibreOffice is a conformance oracle and test
corpus, never a source. In the format notes, a fact marked **`UNVERIFIED` may not be
implemented** (`doc/odt-format.md` §5, `doc/xlsx-format.md`, `doc/docx-format.md`).

**2. ODF semantics are the product.** No Excel-oriented libraries with a translation layer —
the divergence (error model, coercion, empty cells, null-date, reference operators, collation) is
semantic, not syntactic. Format-neutral plumbing (quick-xml, zip, petgraph, chrono) can be lazy;
semantics never are.

## Where to read

| Area | Documents |
|---|---|
| ODF itself | `doc/OpenDocument-v1.4-schema.rng` (Part 3), `doc/OpenDocument-v1.4-os-part4-formula.html` (Part 4) |
| Scope lines | `doc/small-group.md` (functions), `doc/function-coverage.md` (the evidence for going past it), `doc/text-core.md` (text elements), `doc/not-doing.md` (the product line) |
| Format notes | `doc/ods-format.md`, `doc/odt-format.md`, `doc/chart-format.md`, `doc/xlsx-format.md`, `doc/docx-format.md` |
| Core design | `doc/text-layout.md` (layout in the core, Path C), `doc/view-modes.md`, `doc/flat-first.md` (in doubt, write the flat form), `doc/conditional-format.md`, `doc/formula-workbench.md`, `doc/spelling.md`, `doc/pdf-export.md` (paper is ISO 216, A4 by default), `doc/chart-handling.md` |
| Projection and generator | `doc/dsl.md`, `doc/projection-sheet.md`, `doc/projection-text.md`, `doc/generator-spec.md`, `doc/projection-guide.md`, `doc/generator-guide.md`, `doc/editor-setup.md`, `doc/init.md` |
| Import filters | `doc/xlsx-import.md`, `doc/docx-import.md` |
| Shells | `doc/sheet-shell.md` (its "Four surfaces" governs the GTK chrome), `doc/text-shell.md`, `doc/tui-shell.md`, `doc/web-shell.md`, `doc/windows-shell.md`, `doc/macos-shell.md` |
| Parity and licences | `doc/cli-parity-sheet.md`, `doc/cli-parity-text.md`, `doc/feature-matrix.md`, `doc/third-party.md` |

## Commands

```sh
cargo test                                     # the default members: everything without system packages
cargo test -p grind-sheet --test read_values   # one test file
cargo test -- repeated_columns                 # one test by name substring
cargo clippy --workspace --all-targets         # CI gates on it, warnings as errors
cargo fmt --all                                # CI gates on --check
RUSTDOCFLAGS=-D warnings cargo doc --no-deps   # CI gates on it
reuse lint                                     # CI gates on it
```

`grind` is the CLI: `grind <app> <verb>`, plus suite-level verbs that read the kind from the file
(`info`, `convert`, `lint`, `project`) and three with no document (`build`, `test`, `init`).

```sh
cargo run -p grind-cli -- sheet new book.ods
cargo run -p grind-cli -- sheet set book.ods A2 '=[.A1]*2'      # ODF syntax, verbatim
cargo run -p grind-cli -- sheet recalc book.ods
cargo run -p grind-cli -- sheet import-csv book.ods data.csv --locale de-DE
cargo run -p grind-cli -- sheet import book.xlsx book.fods
cargo run -p grind-cli -- text import letter.docx letter.fodt --strict
cargo run -p grind-cli -- text export-pdf report.fodt report.pdf --paper a4
cargo run -p grind-cli -- --format json info book.ods
cargo run -p grind-cli -- build examples/budget.rhai -o book.fods
cargo run -p grind-cli -- test examples/budget.rhai
cargo run -p grind-cli -- lint examples/quote.grind
cargo run -p grind-cli -- lsp
```

The import filters, PDF, spelling and the language server are cargo features, on by default.
CI also builds and tests `-p grind-cli --no-default-features`, because "it still compiles
without it" rots silently.

`examples/sample-sheet.sh` and `examples/sample-text.sh` build a document out of every feature,
through the CLI only, and `cli/tests/cli.rs` runs both. **Add a line there when adding a
capability.**

### Shells

```sh
cargo run -p grind-sheet-gtk -- book.fods                 # needs libgtk-4-dev + libadwaita-1-dev,
cargo run -p grind-text-gtk -- report.fodt                #   and is not a default member
cargo run -p grind-sheet-gtk -- book.fods --render-to /tmp/grid.png   # one frame, then exit
cargo run -p grind-tui -- book.fods                       # one binary, both document types
ui_web/build.sh && ui_web/smoke.sh                        # wasm32 + a matched wasm-bindgen-cli; jsdom
```

`--render-to` is how a custom-drawn widget gets an assertable output; a drawing refactor is
proved one when the frame comes back byte-identical. Not a user feature.

**Windows** (`ui_win32/`) is checked from Linux, because `cargo check` does not link:

```sh
cargo check  -p grind-win32 --target x86_64-pc-windows-msvc
cargo clippy -p grind-win32 --target x86_64-pc-windows-msvc --all-targets -- -D warnings
cargo test   -p grind-win32                                # the portable half, on any host
scripts/run.sh win32                                       # link with cargo-xwin, run under Wine
env -u DISPLAY wine target/x86_64-pc-windows-msvc/debug/grind-win32.exe book.fods --render-to /tmp/f.bmp
```

`cargo build --target x86_64-pc-windows-msvc` fails here; that is expected (no MSVC). **Run Wine
headless when a crash is possible** — WineDbg otherwise opens on whatever desktop is in front of
you. A screenshot under Wine has caught drawing bugs that review and the tests all missed.

**macOS** (`ui_mac/`): the AppKit half type-checks and lints from Linux; linking needs the SDK and
happens only on GitHub's runners (`artifacts.yml`'s `macos` job). `scripts/mac-remote.sh
renders|drive <script>|session` drives the latest built `Grind.app` without compiling. A runner
reports back through **annotations**, which the public API serves without credentials.

```sh
cargo check  -p grind-mac --all-targets --target aarch64-apple-darwin
cargo clippy -p grind-mac --all-targets --target aarch64-apple-darwin -- -D warnings
cargo test   -p grind-mac
```

**`scripts/claude-vm.sh`** runs Claude Code in a microVM whose image carries everything CI needs —
`jing`, `soffice`, GTK with Xvfb, the Windows target with Wine, the wasm toolchain, `reuse`
(`doc/claude-vm.md`). `scripts/claude-vm.sh exec cargo test -p grind-sheet --test kb` runs R2's
schema check rather than skipping it.

## The loops

Correctness is checked against LibreOffice, not our own opinion. Loops C, D and E want `soffice`
on `PATH`; `scripts/soffice-tests.sh` puts `scripts/soffice-docker/soffice` first, which runs the
oracle image pinned by digest in `ci/libreoffice-image`. Loops A and B want a LibreOffice checkout
(`GRIND_LO_CORPUS=/path/to/libreoffice/core`) and skip with a notice without one.

| Loop | Asserts | Where |
|---|---|---|
| A | every corpus `.ods`/`.odt` loads | `sheet/tests/corpus_read.rs`, `text/tests/corpus_read.rs` |
| A′, A″ | every corpus `.xlsx` / `.docx` imports and survives our writer | `xlsx/tests/corpus_read.rs`, `docx/tests/corpus_read.rs` |
| B | formula parse, display round-trip, evaluation matches the cached value | `sheet/tests/corpus_parse.rs`, `corpus_eval.rs` |
| C | write → `soffice` → read back is identical, both directions | `sheet/tests/roundtrip.rs`, `text/tests/roundtrip.rs` |
| D, D″ | our import against the oracle's conversion | `xlsx/tests/loop_d.rs`, `docx/tests/loop_d.rs` |
| E | generated formulas, evaluated by us and by LibreOffice | `sheet/tests/loop_e.rs`, `doc/differential-fuzz.md` |
| F | project → read back is the same model | `sheet/tests/loop_f.rs`, `text/tests/loop_f.rs` |

These never skip: `sheet/tests/kb.rs` (R7, plus R2/R3/R6), `text/tests/libreoffice.rs` (vendored
Writer documents in both forms), `xlsx/tests/ooxmlgen.rs` (Open XML SDK workbooks with a manifest
of expectations; its `PENDING`/`UNSPELLABLE`/`DECIDED_OTHERWISE` tables are checked both ways),
and the function samples (`sheet/tests/function_samples.rs`).

- A `FLOOR` in a loop test is a ratchet: raise it, never lower it.
- Each loop has its documented loosenings and no others; any other exception is a bug in the
  code. A corpus file is never special-cased by name.
- **Adding a function means adding its samples** (`scripts/function-samples.sh`, from odsgen).
  Measure expectations with `table:use-wildcards="true"` in the oracle's document, or
  LibreOffice reads every criterion as a regular expression.

## CI

A binary is built once per platform, and everything that has something to say about it is a
later step in the same job. Debug checks (`ci.yml`, `gtk.yml`, `win32.yml`, `macos.yml`) and
release artifacts (`artifacts.yml`) are the two tiers. `container.yml` builds the images;
`documents.yml`, `screenshots.yml` and `mac-drive.yml` compile nothing and use published
binaries; `mac-session.yml` is dispatch-only; `dependabot-third-party.yml` commits the
regenerated licence list on Dependabot's branches.

Things that bite if forgotten:

- `cargo bloat` rebuilds with debug info and leaves it there, so it is the last step of a job.
- `cargo deb` is passed `--no-build`, or it builds what it packages.
- `push` is restricted to `main`, and every `rust-cache` has `save-if: main`.
- `cli/tests/packaging.rs` reads `artifacts.yml`: every packaged binary needs its `cargo deb` and
  `cargo generate-rpm` lines, its manifest blocks and its `data/`, and the `grind` meta-package
  (`suite/`) must depend on it.
- `core/tests/third_party.rs` rewrites `core/src/third_party/data.rs` and fails when it was stale.

## Architecture

Shared Core / Native Shell. All state and logic live in the core crates; every shell is a
renderer and event forwarder that owns nothing. Rules that are cheap now and expensive to break:

1. **Reads go through `App::get_viewport`.** No getter hands out the whole document.
2. **Undo/redo lives in the core.** `Document::apply` returns the action's inverse.
3. **The core pushes, shells never poll.** `App::mutate` drops the write lock before notifying.
4. **Whatever any GUI can do, the CLI can do** (R9, held by `cli/tests/parity.rs` against the
   two `doc/cli-parity-*.md` files).
5. **No filesystem assumptions.** Every `*_file` has a `*_bytes` twin.
6. **Every feature survives a LibreOffice round-trip** (loop C).
7. **Saving never makes an existing file worse.** Untouched parts and elements are carried as the
   file's own bytes (`grind_core::odf::envelope`, each writer's splicing); what cannot be carried
   is refused with `Error::WouldLose`. `sheet/tests/never_worse.rs` and `text/tests/never_worse.rs`
   hold it; their `REFUSED` lists are a ratchet.

**R8: no document type's vocabulary reaches `grind-core`** (`core/tests/generic.rs`). **R11:
nothing that opens a document depends on `grind-build`** (`build/tests/manifest.rs`).

| Crate | Directory | Holds |
|---|---|---|
| `grind-core` | `core/` | Generic ODF: the package, namespaces, the tolerant reader, `Form`, style primitives, locale, colour, `kind`, line layout, the projection container, lint diagnostics |
| `grind-sheet` | `sheet/` | The spreadsheet: model, column store, ODS reader/writer, number formats, styles, the OpenFormula engine, `App`, projection |
| `grind-text` | `text/` | The word processor: block model, addressing, ODT reader/writer, `App`, projection, Markdown |
| `grind-build` | `build/` | The Rhai generator and its sandbox (`grind build`, `grind test`) |
| `grind-ooxml` | `ooxml/` | The OOXML container layer both import filters share |
| `grind-xlsx` | `xlsx/` | Excel import |
| `grind-docx` | `docx/` | Word import; writes flat ODF and lets `grind-text` read it |
| `grind-print` | `print/` | PDF export and print preview; the only crate with a font stack |
| `grind-spell` | `spell/` | Hunspell dictionaries and the speller |
| `grind-lsp` | `lsp/` | The language server (`grind lsp`) |
| `grind-cli` | `cli/` | The `grind` binary |
| `grind-sheet-gtk`, `grind-text-gtk` | `ui_sheet_gtk/`, `ui_text_gtk/` | The GTK shells |
| `grind-tui` | `ui_tui/` | The terminal shell |
| `grind-web`, `grind-web-print` | `ui_web/`, `ui_web_print/` | The browser shell, and its PDF module loaded on first use |
| `grind-win32` | `ui_win32/` | The Windows shell; its portable half tests on Linux |
| `grind-mac` | `ui_mac/` | The macOS shell; its portable half tests on Linux |
| `grind` | `suite/` | The meta-package, no code |

Facts that are easy to get wrong:

- **Positions are 0-based in the core.** `sheet/src/a1.rs` and `text/src/loc.rs` are the only
  places that convert; a shell never does its own index arithmetic.
- **A document's kind is sniffed from its bytes** (`grind_core::kind`), never its name: the
  readers are tolerant and would hand back an empty document rather than an error.
- `Sheet::used_rows`/`used_cols` cover everything a cell can carry (formula, format, style), and
  the writer emits exactly that rectangle; `Sheet::cell_extent` is the narrower one for `A:A`.
- `grind_text::Document::default()` is one empty paragraph; `Document::empty()` is none and is
  for readers only.
- A number format is an ordered sequence of `Part`s, never a format-code string.
- `CharStyle` is direct formatting only: automatic styles are resolved onto runs, named styles
  stay names.

## Conventions

- **Comments must earn their place.** Say what the code and its names cannot, and stay true when
  other files change: no milestone tags, no "used to", no status ("not yet", "this build"), no
  descriptions of other shells. A doc comment is one to three lines unless the item really needs
  more; a design argument belongs in `doc/`, linked once.
- **`ponytail:` comments** mark deliberate shortcuts with a known ceiling and upgrade path — a
  tracked ledger; read the reason before "fixing" one.
- **`TODO:` comments** are something reported or suspected wrong and not yet reproduced: what is
  checked, what is not, and the suspects. `grep -rn 'TODO:' --include='*.rs'` is the list.

## REUSE / licensing

REUSE compliance is enforced in CI. The licence follows what a file is for (`CONTRIBUTING.md`,
`REUSE.toml`): code **AGPL-3.0-or-later**; prose (`doc/`, the READMEs, `CONTRIBUTING.md`, this
file) **CC-BY-4.0**; what is meant to be copied (`examples/`, `.vscode/`) **MIT**; data and what
`grind init` seeds (fixtures, `cli/templates/`) **CC0-1.0**, declared in `REUSE.toml` rather than
annotated so the documents under test stay unchanged. `sheet/tests/data/kb/**` stays MIT, as
upstream states. A shipped file of ours that is not AGPL goes on `OURS_OTHERWISE` in
`core/tests/third_party.rs`.

```sh
reuse annotate --copyright "Florian Wilhelm <fwilhelm.wgt+github@gmail.com>" \
               --license AGPL-3.0-or-later path/to/file
```

**Never annotate `doc/OpenDocument-v1.4-*` in place** — OASIS specs, redistributed verbatim under
terms that forbid modification; they carry `.license` sidecars. Anything derived from them
carries the OASIS copyright alongside ours.
