<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# grind

An office suite that opens fast, does the parts you actually use, and keeps your files in a
format nobody owns.

Spreadsheets and text documents in OpenDocument — the real thing, not an import filter. The
same document opens four ways: a **command line**, a **terminal UI**, a **desktop window**, and
a **browser tab that runs entirely on your own machine**. One core underneath all four, so they
cannot disagree with each other.

## Read this first

**This is software I wanted, built the way I want it.** It is not a product, not a LibreOffice
replacement for anybody but me, and not a community effort taking feature requests. The feature
list is short *on purpose*, and [`doc/not-doing.md`](doc/not-doing.md) is the document that says
where it stops. If the thing you need is on that list, this will never do it — that is a
decision, not a gap. There is no guarantee it does what you need, and no guarantee that what
works today still works next month.

Take it, use it, fork it. Just do not expect it to fit your work because it fits mine.

**How far along each part is, plainly:**

| | |
|---|---|
| **The spreadsheet** | Furthest along by a long way, and the part I use daily. Formulas, number formats, styling, charts, named ranges, multiple sheets — in all four front ends. |
| **The word processor** | Early. It reads, writes and edits real documents — headings, lists, bookmarks, character formatting, images — and it has **no page model**: a document is one reflowed column. Plenty of people would call that a rich-text editor rather than a word processor, and today they are right. |
| **Anything else** | Presentations, drawings, a database: not built, not scheduled, and quite possibly never. The suite is *shaped* so a third document type could be added without rewriting the first two. That is the whole claim. |

## Why this exists

LibreOffice is great. It is the reason OpenDocument is a real format and not a standards
document, and it does everything — which is the problem. Thirty years of menus, four toolbars,
dialogs inside dialogs, and somewhere in there the six things I actually do in a spreadsheet:
type numbers, sum a column, format it so it reads properly, name a range, look at what a
formula is doing, save it.

So this is those six things rebuilt from scratch, on my own terms:

- **ODF-native, because free formats matter.** Not "imports .ods" — the file on disk *is* the
  model. OpenFormula semantics, ODF's error values, ODF's number formats, ODF's references.
  Nothing is translated through somebody else's spreadsheet dialect on the way in or out, so
  nothing is lost in translation. Everything written validates against the OASIS schema.
- **A feature list that ends.** Legacy features almost nobody uses are simply not here: no
  macros, no extensions, no pivot tables, no change tracking, no OLE embedding, no solver, no
  scenarios, no sparklines. A spreadsheet you can hold in your head is the goal, not a milestone
  on the way to a bigger one.
- **Small enough to trust.** There is no scripting host and no extension API, so a document this
  opens cannot execute anything — not by design decision alone, but because the machinery does
  not exist and is not allowed to.

It is not a port of LibreOffice and contains none of its code. It implements the OASIS
specifications, and uses LibreOffice as an oracle and a test corpus — every formula it evaluates
is checked against LibreOffice's answer, and every file it writes is opened by LibreOffice and
read back.

Nearly all of it is written with an AI agent, which is the other experiment: the rules live in
checked-in documents, and tests fail the build when the code and the documents disagree.

## What the spreadsheet can do

Cells, types, and **all 110 functions of OpenFormula's Small Group** (plus `ROW` and `COLUMN`,
which earned their place) — `grind sheet functions` prints the list. Multiple sheets, named
ranges and named expressions, filters, undo/redo. **Number formats**, so a date prints as a date
and a German document's `1.234,50` stays `1.234,50`. **Cell styling** — weights, colours,
borders, alignment. Column widths and row heights, hidden rows and columns. **Charts** — bar,
line and pie — that survive a round trip through LibreOffice like everything else.

**CSV and TSV, in and out**, which is the one non-ODF format here. The delimiter is read out of
the file rather than out of its name, so a comma file, a German semicolon file and a tab file
all just open; `--locale de-DE` reads `1.234,50` as a number. What a field means is the same
rule as typing it into a cell, with the guards a real file needs — `007` stays a product code
instead of becoming 7, `NaN` stays somebody's name, and a leading `=` stays text unless you ask
for a formula. Dates are ISO only and behind a flag, because `15/03/2026` and `03/15/2026` are
the same characters meaning two different days.

Not there yet: sort (it needs a locale-collation decision first), find/replace, freeze panes,
printing. Reading `.xlsx` is planned; *writing* it never is. Fonts are a named gap — nothing
here picks a typeface for you yet.

## What the word processor can do

Paragraphs, headings and lists; bookmarks; outline navigation; images; find and replace; word
count. **Character formatting** — bold, italic, underline, strike, family, size, colour,
highlight — which round-trips through LibreOffice character by character.

Addressing is the unusual part, and it is what makes a document scriptable: a place is `p12`
(the twelfth block), `#intro` (a bookmark), or `§2.1.3` (an outline path), optionally plus a
character offset. The last two keep meaning the same place after you edit somewhere else in the
document.

Page size, margins, headers, footers and page breaks are read, kept and written back untouched —
they are simply never *drawn* as pages. **Line** layout, though, lives in the shared core, so
`j`, `k`, Home and End mean exactly the same thing in every front end and the command line can
answer them too; each front end supplies only font metrics. What is missing beyond pages: tables,
footnotes, fields, style definitions, and right-to-left layout.

## Four ways in

Every capability lives in one Rust core, and each front end is a window onto it that owns
nothing. The rule — enforced by a test, not by good intentions — is that **anything a window can
do, the command line can do**.

### The command line

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
grind sheet import-csv book.ods data.csv        # delimiter sniffed; "-" reads a pipe
grind sheet export-csv book.ods --out out.csv   # what each cell shows, quoted where needed
```

Cells are addressed the way ODF references them, minus the brackets — `A1`, `$B$7`, `Data.B2`,
`'Q3 Actuals'.A1:.C9`. `grind text` is the same shape for documents: `type`, `insert`, `style`,
`outline`, `find`, `replace`, `words`. `grind info`, `grind lint` and `grind convert` work on
either kind and decide which by reading the file, never by its name.

`--format json` makes every command machine-readable, `--dry-run` applies a change and writes
nothing, and `--session` carries the spreadsheet's undo history across invocations — which
together make this a reasonable thing to point a script, a CI job or an agent at.
[`doc/cli-recipes-sheet.md`](doc/cli-recipes-sheet.md) has worked examples: CSV import, a PMT
model, a CI gate on error cells, git diffs of `.ods` files.

### The terminal

```sh
grind-tui book.ods      # the spreadsheet
grind-tui report.fodt   # the word processor
grind-tui --text        # a new document, empty
```

One binary, both document types, decided from the file's own bytes. No system packages needed.
Vi-style modes in both: **Normal** navigates (`hjkl`/arrows, `g`/`G`, `Ctrl-f`/`Ctrl-b`),
**Visual** (`v`) selects, **Insert** edits, `:` opens a command line. `j` and `k` move by
*wrapped line* in a document rather than by paragraph.

**Formatting is typed as markdown and drawn as formatting.** `**bold**`, `*italic*`,
`__underline__` and `~~struck~~` become the document's own formatting as the closing marker
lands — the markers are erased, and what stays is bold, italic, underlined or struck in the
terminal's own attributes. `` `code` `` sets a monospace family, three backticks open a code
block, `# ` makes a heading and `- ` a list item. Over a selection each is a single key: `*`,
`/`, `_`, `~`, `-`. The spreadsheet gets `:align`, `:color`, `:fill`, `:format` and the rest,
draws each cell's own styling, folds away rows a filter hides, and yanks a range as
tab-separated text. `:help` lists every key without leaving the document.

### The desktop

```sh
grind-sheet-gtk book.ods        # or a .fods; with no file, an empty document
grind-text-gtk  report.fodt     # or a .odt; the word processor
```

GTK 4 and libadwaita — a GNOME application, keyboard first. Two binaries rather than one with a
mode, because that is how a desktop associates a file type with an application.

The spreadsheet window is the mature one: type in cells, type formulas in the A1 form you
already know (with point mode, autocomplete and a signature hint as you go), select, copy,
paste, undo, resize columns by dragging or double-clicking to fit, zoom with Ctrl and the wheel.
There is no ribbon: a header bar, one format bar over the selection, context menus, and
**Ctrl+K** for every other verb in the application. The word processor's window is deliberately
minimal — it opens, draws, moves by line, types, formats a selection, saves and undoes, with an
outline and a go-to box for `p12` / `#intro` / `§2.1.3`.

**macOS and Windows clients are planned and do not exist.** Nothing is written, nothing is
scheduled. Everything here is built and tested on Linux; the command line and the terminal UI are
portable Rust with no platform-specific dependencies and will probably build elsewhere today, but
nothing verifies that yet, so treat it as untested rather than supported.

### The browser

The same core compiled to WebAssembly. **No server, no upload, no account** — the page is a
static bundle and the document never leaves your machine, because there is nowhere to send it.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version "$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock \
  | sed -n 's/^version = "\(.*\)"/\1/p' | head -n1)"
./ui_web/build.sh                                    # writes ui_web/dist
python3 -m http.server --directory ui_web/dist 8000  # a module needs http, not file://
```

It is a web page, not a window pretending to be one: one bar of verbs, one row of tools for
whichever document is open, and **Ctrl+K** for everything else — a searchable list of every
command, which doubles as the go-to box. Type `B12`, a sheet's name, a defined name or a heading
and it takes you there. Drop a file on the page to open it.

Both document types in one bundle. The spreadsheet has formatting, clipboard through the
browser's own, the document's column widths and row heights, charts drawn as SVG, and sheets you
can add, rename and delete. The word processor has a selection, character formatting, headings,
its own pictures, and the outline in the palette.

## Three things the big suites do not do

### 1. View source

Every front end can show you the document as **plain text you can read** — `Ctrl+Shift+U` in
either desktop window, `:source` in the terminal, *Show the source* in the browser,
`grind sheet project` on the command line:

```kdl
grind spreadsheet

sheet Sales {
    at A1 {
        row Region Q1   Q2
        row North  4200 4800
        row South  3100 3300
    }
    cell B4 "=SUM([.B2:.B3])"
}
```

That is not a preview. It is a **third file format**, `.grind`, beside the zipped `.ods` and the
flat `.fods` — the same document, spelled as text. Every front end opens one, `grind convert`
moves between all three, and it is bijective with the model: read it, edit it, write it back, and
nothing has moved. Edit one cell and **one line of the file changes**, with your comments and
your hand alignment still where you left them. A formula does not need its answer, so a model can
be written by hand without doing any of its arithmetic — `grind sheet recalc` fills the numbers
in afterwards.

Which makes two things possible that a binary spreadsheet cannot have. A change becomes
**reviewable**: a rate rising from £62 to £65 is one line of diff, and recalculating shows
exactly which seventeen cells moved. And a document becomes **checkable in CI**: `grind lint`
finds a cached total that disagrees with its own formula, a formula naming a deleted sheet, or
anything the plain-text form would not carry — and exits non-zero on an error.

### 2. Formulas in plain English

The formula bar shows what a cell *does*, not what it stores:

```
=RATE(A1;-100;1000;0;0;0.05)
Interest Rate(Number Of Periods: A1; Payment: -100; Present Value: 1000; …)
```

Click the bar and it turns straight back into the real formula for editing. The file is never
touched, nothing is renamed, and the document stays exactly as compatible as it was. The ⓘ button
unfolds a nested formula one argument per line; `grind sheet fmt --friendly` does the same thing
in a terminal.

`Ctrl+Shift+F` lists **every formula in the document**, searchable by address, by formula text or
by function name, each one clicking through to its cell — including the arithmetic, because
`=A1/2` is as much a calculation as `=SUM(A1:A9)` and it is usually the one you were looking for.

### 3. A document you can generate

When the repetition gets tiring, a script writes the document instead of you:

```console
$ grind build examples/timesheet.rhai -o month.fods
```

One line per *kind* of cell rather than one per cell, out of a JSON file that somebody who has
never read a line of code can edit. The language is small and deliberately caged: **no
filesystem** beyond one data directory you name, no network, no clock, no randomness, and
everything bounded, so a script that will not terminate is an error with a line number rather
than a hang.

This is emphatically **not macros.** A generator is a source file that lives *beside* a document
and runs when you type a command. Nothing that *opens* a document can evaluate anything — that is
a rule with a test behind it, checked against every package in the workspace. The arrow points one
way: a script produces a document and is never recovered from one.

Two guides, each built on a real example in this repository:
[`doc/projection-guide.md`](doc/projection-guide.md) for writing a document by hand, and
[`doc/generator-guide.md`](doc/generator-guide.md) for generating one, with
[`doc/editor-setup.md`](doc/editor-setup.md) for editor support.

## Your files stay yours

Writing **changes as little XML as it can**. Setting one cell in a 482-line LibreOffice file
changes one element and leaves every other byte alone, indentation included — so a `.fods` lives
in git the way a source file does, and opening a document just to look at it is not a commit. A
new file is thirteen lines rather than five hundred, because it carries only the boilerplate it
uses.

And that is the **default**, not a setting to hunt for: in doubt this suite writes the flat form,
because the property above is worth nothing if the file it applies to is a zip. Save dialogs lead
with `.fods` / `.fodt`, a new document is flat, and naming `book.ods` is how you ask for a
package. No document is ever converted behind your back.

Strictness on the way out, tolerance on the way in: everything written is valid ODF, and
everything LibreOffice writes reads — unknown elements and attributes included, kept intact
rather than dropped.

## How it is checked

Correctness is not my opinion about it. Four checks run in CI, none of them grading their own
homework:

- Every spreadsheet in **LibreOffice's own test corpus** loads — 359 of them, plus three it
  declines because they are password-protected. On the text side, 1755 Writer documents.
- Every formula in that corpus **parses, prints back identically, and evaluates to the value
  LibreOffice cached** for it, with a ratchet that can only go up.
- Whatever this writes, **LibreOffice reads back unchanged — and the reverse.**
- Formulas generated from the function catalogue are **evaluated by both** and compared, against a
  LibreOffice pinned by digest so the result means something.

Plus a set of documents that must always load, a set LibreOffice Writer wrote — in both forms of
the same file, so the zipped reader and the flat reader are held to one answer — and a check that
everything written validates against the OASIS RELAX NG schema. Those test documents are CC0: a
fixture is data, and the point of one is that anybody can take it.

## Getting it

There are **no releases yet**. Build it, or take a CI artifact.

Install Rust via [rustup](https://rustup.rs/) rather than your distribution's package — this
tracks current stable, which is what CI builds against. The desktop windows also need GTK 4 and
libadwaita development headers; nothing else does.

```sh
# Debian / Ubuntu
sudo apt-get install -y --no-install-recommends libgtk-4-dev libadwaita-1-dev

# Fedora
sudo dnf install -y gtk4-devel libadwaita-devel
```

```sh
cargo install --path cli                # the `grind` command line
cargo run -p grind-tui -- book.fods     # the terminal UI
cargo run -p grind-sheet-gtk -- book.fods
cargo run -p grind-text-gtk  -- report.fodt
```

`.deb` and `.rpm` packages for all four binaries are built on every push and kept as workflow
artifacts. The command line is also a container image, about as small as one gets:

```sh
podman run --rm -v "$PWD:/work:z" ghcr.io/fwilhe2/grind:latest /grind info /work/book.fods
```

To see what a build can actually do, `examples/sample-sheet.sh` and `examples/sample-text.sh`
build a document out of **every feature it has**, through the command line and nothing else:

```sh
cargo build
GRIND=target/debug/grind examples/sample-sheet.sh /tmp/demo
cargo run -p grind-sheet-gtk -- /tmp/demo/sample.fods
```

```sh
cargo test    # everything but the GTK windows
```

## Reading further

| Document | What |
|---|---|
| [`doc/not-doing.md`](doc/not-doing.md) | The feature line — never, not yet, and where what exists stops |
| [`doc/cli-recipes-sheet.md`](doc/cli-recipes-sheet.md) | Worked command-line scripts |
| [`doc/projection-guide.md`](doc/projection-guide.md) | Writing a spreadsheet by hand, as plain text that reviews like code |
| [`doc/generator-guide.md`](doc/generator-guide.md) | Generating one from a script, and from data nobody has to be a programmer to edit |
| [`doc/editor-setup.md`](doc/editor-setup.md) | VS Code for both, and what each extension actually does |
| [`doc/small-group.md`](doc/small-group.md) | The 110 functions, and where each is defined |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | The clean-room rule, and how to work on this |

The rest of `doc/` is the design record — why each piece is shaped the way it is, written for
whoever works on the code. `doc/plan.md` is the map of it.

## License

**AGPL-3.0-or-later.** Full text in [`LICENSES/`](LICENSES/).

If you modify this and offer it to others over a network, §13 requires you to publish your
source. That is deliberate: a spreadsheet core is exactly the thing someone embeds in a hosted
service, and plain GPL would ask nothing of them.

The repository is [REUSE](https://reuse.software) compliant — every file carries its copyright
and license, machine-readably (`reuse lint`).

The two OASIS specifications under `doc/` are **not** AGPL and **not** open source. They are
redistributed verbatim under the OASIS IPR Policy, which permits copying but forbids modification
of any kind — including adding an SPDX header. They are marked with `.license` sidecar files for
that reason; do not annotate them in place.

## Trademarks

LibreOffice is a registered trademark of [The Document
Foundation](https://www.documentfoundation.org/). OpenDocument and ODF are trademarks of
[OASIS](https://www.oasis-open.org/). GNOME is a trademark of the GNOME Foundation.

This project is not affiliated with, endorsed by, or sponsored by any of them. LibreOffice is
named here only to describe what this software does — which documents it reads, which
implementation it is tested against, and how the two differ — and every such use is descriptive,
never a claim of origin. No trademark is used in the name of this project, its binaries, its
icon, or its packaging.
