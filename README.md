<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# grind

An office suite that opens fast, does the parts you actually use, and keeps your files in a
format nobody owns.

One spreadsheet today, `grind sheet`. A word processor, `grind text`, is being built beside
it — [`doc/suite.md`](doc/suite.md) is the plan and its rules.

**And one thing said plainly, because it decides whether this is for you: `grind text` has no
pages today.** It shows a document as one reflowed column. Page size, margins, headers, footers
and page breaks are read, kept and written back untouched — they are just never *drawn* as
pages, and printing goes through the platform. Plenty of people will say that makes it a
rich-text editor rather than a word processor, and today they are right. Pagination has a named
gate rather than a refusal ([`doc/not-doing.md`](doc/not-doing.md) §2), and the larger question
underneath it — how much of layout belongs in the shared core rather than in each UI — is being
decided in the open in [`doc/text-layout.md`](doc/text-layout.md).

## Why this exists

LibreOffice is great. It is the reason OpenDocument is a real format and not a standards
document, and it does everything — which is the problem. Thirty years of menus, four
toolbars, dialogs inside dialogs, and somewhere in there the six things I do in a
spreadsheet: type numbers, sum a column, format it so it reads properly, name a range, look
at what a formula is doing, save it.

So this is my attempt to rebuild those six things from scratch, on my own terms:

- **ODF-native, because free formats matter.** Not "imports .ods". The file on disk *is* the
  model — OpenFormula semantics, ODF's error values, ODF's number formats, ODF's
  references. Nothing is translated through somebody else's spreadsheet dialect on the way
  in or out, so nothing is lost in the translation. Everything written validates against the
  OASIS schema.
- **A feature list that ends.** [`doc/not-doing.md`](doc/not-doing.md) is a product document:
  what this will never do, what it does not do yet, and where each capability that exists
  stops. A spreadsheet you can hold in your head is the point, not a milestone on the way to
  a bigger one.
- **An experiment in how software gets built.** Nearly all of this is written with an agent,
  and the repository is arranged so that works: the rules are checked-in documents, and tests
  fail the build when the code and the documents disagree. Correctness is not my opinion
  either — every formula this thing evaluates is checked against LibreOffice's answer, and
  every file it writes is opened by LibreOffice and read back.

It is not a port of LibreOffice and contains none of its code. It implements the OASIS
specifications, and uses LibreOffice as an oracle and a test corpus.

## Where it is

**Phases 0–8 done, phase 9 (the shells) in progress.** It reads and writes ODF spreadsheets
in both forms, `.ods` and flat `.fods`. All 361 documents in LibreOffice's own Calc test
corpus load — the three it declines are password-protected — and documents written here
survive a round trip through LibreOffice unchanged, checked on every push.

It evaluates **all 110 of OpenFormula's Small Group functions**, keeps **number formats** so
a date prints as a date and a German document's `1.234,50` stays that, and **cell styling** —
weights, colours, borders, alignment. It has named ranges, multiple sheets, undo, column
widths and row heights.

No fonts yet, and the list of what is deliberately missing is a document rather than an
excuse: [`doc/not-doing.md`](doc/not-doing.md).

## The window

```sh
cargo run -p grind-sheet-gtk -- book.ods        # or a .fods; with no file, an empty document
```

GTK 4 and libadwaita — a GNOME application, keyboard first. Type in cells, type formulas in
the A1 form you already know (with point mode, autocomplete and a signature hint as you go),
select, copy, paste, undo, resize columns by dragging or double-clicking to fit, zoom with
Ctrl and the wheel.

Two things it does that the big one does not:

**Formulas in plain English.** The formula bar shows what a cell *does*, not what it stores:

```
=RATE(A1;-100;1000;0;0;0.05)
Interest Rate(Number Of Periods: A1; Payment: -100; Present Value: 1000; …)
```

Click the bar and it turns straight back into the real formula for editing — the file is
never touched, the names are never renamed, and everything stays exactly as compatible as it
was. The ⓘ button unfolds a nested formula one argument per line.

**Find everything that is calculated.** `Ctrl+Shift+F` lists every formula in the document,
searchable by address, by formula text or by function name, each one clicking through to the
cell. Including the arithmetic — `=A1/2` is as much a calculation as `=SUM(A1:A9)`, and it is
usually the one you were looking for.

## The other three front ends

Every capability lives in one Rust core; each front end is a window onto it that owns
nothing. The rule — enforced by a test, not by intention — is that **anything a window can
do, the command line can do**.

**The command line** (`grind`), which is the whole feature set:

```sh
grind sheet new book.ods
grind sheet set book.ods A1 1
grind sheet set book.ods A2 2
grind sheet set book.ods A3 '=SUM([.A1:.A2])'   # OpenFormula syntax, stored verbatim
grind sheet recalc book.ods
grind sheet format book.ods A3 currency --symbol '€' --grouping
grind sheet style book.ods A1 --bold --background '#dddddd'
grind sheet name book.ods total A1:A2           # a named range, so formulas say what they mean
grind sheet calculations book.ods               # every computed cell, and what it calls
grind sheet view book.ods A1:A3                 # tab-separated, pipes into anything
```

Cells are addressed the way ODF references them, minus the brackets — `A1`, `$B$7`,
`Data.B2`, `'Q3 Actuals'.A1:.C9`. `--format json` makes every command machine-readable and
`--session` carries undo across invocations, which together make this a reasonable thing to
point a script — or an agent — at. [`doc/cli-recipes.md`](doc/cli-recipes.md) has worked
examples: CSV import, a PMT model, a CI gate on error cells, git diffs of `.ods` files.

`grind sheet format`'s `--locale` decides the decimal and grouping characters (`1,234.50` vs.
`1.234,50`). Leave it off and the app falls back to `$GRIND_LOCALE`, then
`$XDG_CONFIG_HOME/grind/locale` (a bare tag like `de-DE`, nothing else in the file), then no
locale at all. The GTK window's format picker uses the same fallback when its locale field is
left blank.

**The terminal** (`grind-tui`, no system packages needed):

```sh
cargo run -p grind-tui -- book.ods
```

Vi-style modes: **Normal** navigates (`hjkl`/arrows, `g`/`G`, `Ctrl-f`/`Ctrl-b`), **Insert**
(`i`/`a`/`c`) edits, `:` opens a command line (`:w`, `:q`, `:recalc`, or a bare address to
jump). `grind-tui --help` has the full key list.

**The browser** (`grind-web`, the same core as WebAssembly — no server, the document never
leaves your machine):

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version "$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock \
  | sed -n 's/^version = "\(.*\)"/\1/p' | head -n1)"
./ui_web/build.sh                                    # writes ui_web/dist
python3 -m http.server --directory ui_web/dist 8000  # a module needs http, not file://
```

Open, edit, formulas, undo, sheets, save as a download. No point mode, no styling controls,
one column width for everything.

## Your files stay yours

Writing **changes as little XML as it can**. Setting one cell in a 482-line LibreOffice file
changes one element and leaves every other byte alone, indentation included — so a `.fods`
lives in git the way a source file does, and opening a document to look at it is not a
commit. A new file is thirteen lines rather than five hundred, because it carries only the
boilerplate it uses.

Strictness on the way out, tolerance on the way in: everything written is valid ODF, and
everything LibreOffice writes reads — unknown elements and attributes included, kept intact
rather than dropped.

## What it will and will not do

**In:** multiple sheets · cell values and types · OpenFormula **Small Group** (110 functions,
[`doc/small-group.md`](doc/small-group.md)) · formatting and number formats · sort and filter ·
find/replace · freeze panes · one chart type · CSV · print to PDF.

**Out, permanently:** macros · extensions · pivot tables · change tracking · OLE embedding ·
xlsx *writing* · scenarios · solver · sparklines · OpenFormula Large Group.

The "out" list is the product. Items move off it one at a time by explicit decision, and only
if they survive a round trip through LibreOffice.

## How it is checked

Four loops, all of them running in CI, none of them grading their own homework:

| | Asserts |
|---|---|
| **A** | every document in LibreOffice's Calc corpus loads |
| **B** | formulas from that corpus parse, round-trip, and evaluate to the value LibreOffice cached |
| **C** | what we write, LibreOffice reads back unchanged — and the reverse |
| **E** | formulas generated from the function catalog, evaluated by us and by LibreOffice, compared |

Plus eight named documents that must always load, vendored in the repository, and a check
that everything written validates against the OASIS RELAX NG schema.

## Building

Install Rust via [rustup](https://rustup.rs/) rather than your distribution's package —
this tracks current stable, which is what CI builds against. The GTK shell also needs
GTK 4 and libadwaita development headers:

```sh
# Debian / Ubuntu
sudo apt-get install -y --no-install-recommends libgtk-4-dev libadwaita-1-dev

# Fedora
sudo dnf install -y gtk4-devel libadwaita-devel
```

```sh
cargo test                       # everything but the GTK shell
cargo test -p grind-sheet-gtk          # its widget-free half: geometry, keys, edit state
```

The corpus tests want a LibreOffice checkout and skip with a notice without one:

```sh
GRIND_LO_CORPUS=/path/to/libreoffice/core cargo test
```

`examples/sample.sh` builds a document out of every feature this build has, through the CLI
and nothing else — which also makes it the most interesting thing to open in the window:

```sh
cargo build
GRIND=target/debug/grind examples/sample.sh /tmp/demo
cargo run -p grind-sheet-gtk -- /tmp/demo/sample.fods
```

## Reading further

| Document | What |
|---|---|
| [`doc/plan.md`](doc/plan.md) | The requirements, the phases, and what each one has to prove |
| [`doc/not-doing.md`](doc/not-doing.md) | The feature line — never, not yet, and where what exists stops |
| [`doc/cli-parity-sheet.md`](doc/cli-parity-sheet.md) | Every core capability against the command that reaches it |
| [`doc/cli-recipes.md`](doc/cli-recipes.md) | Worked scripts |
| [`doc/small-group.md`](doc/small-group.md) | The 110 functions, from Part 4 §2.3.2 |
| [`doc/ods-format.md`](doc/ods-format.md) | Clean-room notes on what LibreOffice actually does, cited `file:line` |
| [`doc/gtk-shell.md`](doc/gtk-shell.md) | The GTK shell, milestone by milestone |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | The clean-room rule, and how to work on this |
| `doc/OpenDocument-v1.4-schema.rng`, `doc/OpenDocument-v1.4-os-part4-formula.html` | The OASIS specifications this is built from |

## License

**AGPL-3.0-or-later.** Full text in [`LICENSES/`](LICENSES/).

If you modify this and offer it to others over a network, §13 requires you to publish your
source. That is deliberate: a spreadsheet core is exactly the thing someone embeds in a
hosted service, and plain GPL would ask nothing of them.

The repository is [REUSE](https://reuse.software) compliant — every file carries its
copyright and license, machine-readably (`reuse lint`).

The two OASIS specifications under `doc/` are **not** AGPL and **not** open source. They are
redistributed verbatim under the OASIS IPR Policy, which permits copying but forbids
modification of any kind — including adding an SPDX header. They are marked with `.license`
sidecar files for that reason; do not annotate them in place.

## Trademarks

LibreOffice is a registered trademark of [The Document
Foundation](https://www.documentfoundation.org/). OpenDocument and ODF are trademarks of
[OASIS](https://www.oasis-open.org/). GNOME is a trademark of the GNOME Foundation.

This project is not affiliated with, endorsed by, or sponsored by any of them. LibreOffice is
named here only to describe what this software does — which documents it reads, which
implementation it is tested against, and how the two differ — and every such use is
descriptive, never a claim of origin. No trademark is used in the name of this project, its
binaries, its icon, or its packaging.
