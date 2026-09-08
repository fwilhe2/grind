<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The feature matrix — every capability, in every client

> **Whatever any GUI can do, the CLI can do. A UI-only feature is a bug.**
> — `doc/plan.md` rule 4
>
> **R10 allows per-shell feature gaps and requires them to be named.**
> — `doc/suite.md`

Rule 4 is checked mechanically, and only down one column: `cli/tests/parity.rs` reads
`sheet/src/lib.rs` and `text/src/lib.rs` against `doc/cli-parity-sheet.md` and
`doc/cli-parity-text.md`, so **the CLI's column cannot silently lose a row**. Nothing checks the
other five. Each shell names its own gaps in its own document, which is the right place for the
*reason* — and it is the wrong place to see that `grind-win32` cannot bold a cell while
`grind-tui` can, because no reader holds five gap lists in their head at once.

This file is that cross-cut: one row per capability, one column per client, read out of the
code rather than out of the five shell documents. It is **descriptive, not normative** — where
it disagrees with `doc/sheet-shell.md`, `doc/text-shell.md`, `doc/tui-shell.md`,
`doc/web-shell.md` or `doc/windows-shell.md`, those are right and this is stale. §10 says how to
re-derive it, and why that step is worth keeping.

## 1. The clients

| Column | Crate | Binary | Document types | Command surface |
|---|---|---|---|---|
| **CLI** | `grind-cli` | `grind` | both, kind read from the file | subcommands (`grind <app> <verb>`) |
| **Sheet GTK** | `grind-sheet-gtk` | `grind-sheet-gtk` | spreadsheet only | header bar, format bar, context menus, **Ctrl+K palette** |
| **Text GTK** | `grind-text-gtk` | `grind-text-gtk` | text only | header bar, format bar, one primary menu |
| **TUI** | `grind-tui` | `grind-tui` | both, one binary | vi keys and a `:` command line |
| **Web** | `grind-web` | one wasm bundle | both, one bundle | verb bar, one tool row per type, **Ctrl+K palette** |
| **Win32** | `grind-win32` | `grind-win32.exe` | both, one binary | **menu bar**, context menus, format strip |

The GTK pair is two binaries **by decision** (`doc/suite.md`, "One binary per document type —
for GTK only"): a `.desktop` file's `MimeType=` is per application. Everywhere else one binary
opens either kind, dispatched on `grind_core::kind` from the file's bytes.

**Legend.** ● built · ◐ partial, see the notes under the table · ○ absent, and deferred by
decision · — not applicable to this client.

## 2. Suite-wide

| | CLI | Sheet GTK | Text GTK | TUI | Web | Win32 |
|---|---|---|---|---|---|---|
| Opens a spreadsheet | ● | ● | — | ● | ● | ● |
| Opens a text document | ● | — | ● | ● | ● | ● |
| Reads all three forms (`.ods`/`.fods`, `.odt`/`.fodt`, `.grind`) | ● | ● | ● | ● | ● | ● |
| Writes all three forms | ● | ● | ● | ● | ● | ● |
| Flat-first default (`doc/flat-first.md`) | ● | ● | ● | ● | ● | ● |
| New, empty document | ● | ● | ● | ◐ ᵃ | ○ | ● |
| Undo / redo — spreadsheet | ◐ ᵇ | ● | — | ● | ● | ● |
| Undo / redo — text | ○ ᶜ | — | ● | ● | ● | ● |
| Code view — the projection, read-only (D9) | ● | ● | ● | ● | ● | ● |
| Check Document — `lint` findings, each a jump (D6) | ● | ● | ● | ● | ● | ● |
| Go to an address | ● | ● | ● | ● | ● | ● |
| Key list / help | ● | ● | ○ | ● | ◐ ᵈ | ● |
| About / build stamp | ● | ● | ● | ○ | ○ | ● |
| Recent files | — | ● | ○ | ○ | ○ | ○ |
| Opening the *other* document kind | ● ᵉ | ○ ᶠ | ● ᵉ | ● | ● | ● |
| Assertable headless output | stdout | `--render-to` PNG | `--render-to` PNG | `TestBackend` | `smoke.js` (jsdom) | `--render-to` BMP |
| `.deb` + `.rpm` | ● | ● | ● | ● | — | — |
| Accessibility floor | — ᵍ | announce | announce | terminal | ARIA labels | system caret |

ᵃ `grind-tui --sheet` / `--text` starts an empty document; there is no in-session "new".
ᵇ Only with `--session`, which carries `grind_sheet::Action` between invocations.
ᶜ **Named**: `grind_text::Action` is not serialisable, so there is no text session
(`doc/cli-parity-text.md`, "History"). Every shell can undo a text edit in process.
ᵈ The palette shows each verb's key beside it; there is no shortcuts window.
ᵉ The four clients marked plainly ● simply open it — one binary, both kinds. The CLI routes on
`grind_core::kind` at the suite level (`info`, `lint`, `convert`); `grind-text-gtk` cannot open
one and says so, raising a banner — *"This is a spreadsheet"* + **Open in Sheet**, which launches
the other binary.
ᶠ **A real asymmetry, and the one direction of the handoff that is missing.**
`grind-sheet-gtk` opening a `.fodt` raises nothing: the ODS reader is tolerant by construction,
so it returns a document with no sheets in it and the window reports whatever the first read of
sheet 0 says (`grind sheet view report.fodt` prints `no such sheet: 0`). `grind_core::kind` is
the check it lacks, and its twin's banner is the shape of the answer.
ᵍ A pipe is the accessible surface, which is `doc/view-modes.md` §4.6's argument for why the
CLI matters most exactly where a GUI's whole output is colour.

## 3. Spreadsheet — navigation and selection

| | CLI | Sheet GTK | TUI | Web | Win32 |
|---|---|---|---|---|---|
| Move by cell, row, page, document | — | ● | ● | ● | ● |
| Jump to the edge of a block (Ctrl+arrow) | — | ● | ○ | ○ | ● |
| Jump to the start or end of the sheet | — | ● | ● | ● | ● |
| Select a rectangle | ● ᵃ | ● | ● | ● | ● |
| Select whole rows / columns from a header | ● ᵃ | ● | ○ | ○ | ● |
| Select the whole sheet | ● ᵃ | ● | ○ | ● | ● |
| Name box / address field | ● | ● | ● | ● | ● |
| Go to a defined name | ● | ● | ○ | ● | ● |
| Skip a hidden or filtered row while moving | — | ○ ᵇ | ● ᶜ | ○ | ● |
| Sheet switching | address | tab strip | `:sheet` | tab strip | menu + Ctrl+PgUp/PgDn |
| Zoom | — | ● | ○ | ○ | ○ |

ᵃ A range is an argument, not a gesture: `A1:C9`, `A:A`, a sheet-qualified form.
ᵇ **Named** in `doc/sheet-shell.md`: `keymap.rs` is pure and knows nothing about the document,
so skipping means handing it the hidden set.
ᶜ Rows folded away by a filter are gone from the view entirely, so there is nothing to step onto.

## 4. Spreadsheet — editing and formulas

| | CLI | Sheet GTK | TUI | Web | Win32 |
|---|---|---|---|---|---|
| The typing rule (`=` formula, `'` text, empty clears) | ● | ● | ● | ● | ● |
| In-cell editor | — | ● | ○ ᵃ | ○ ᵃ | ● |
| Formula bar | — | ● | ◐ ᵃ | ● | ● |
| Clear a cell or a range | ● | ● | ● | ● | ● |
| Clear only the formula, keeping the value | ● | ○ | ○ | ○ | ○ |
| Paste a rectangle of tab-separated rows | ● | ● | ● | ● | ● |
| System clipboard (cut / copy / paste) | — | ● | ◐ ᵇ | ● | ● |
| Copy Value — the formatted result, not the formula | ● | ● | ○ | ○ | ○ |
| Fill down / fill right | ● | ● | ○ | ● | ○ |
| Fill one cell across a rectangle (references shifted) | ● | ● | ○ | ○ | ○ |
| Recalculate | ● | ● | ● | ● | ● |
| Stale-value warning | ● | ● | ● | ● | ● |
| Evaluate a formula without storing it | ● | ● ᵈ | ○ | ○ | ○ |
| Selection arithmetic — Sum, Count, Average | ● ᵉ | ● | ○ | ○ | ● |
| Autocomplete while typing a formula | ● ᶜ | ● | ○ | ○ | ● |
| Signature hint for the call the caret is in | ● ᶜ | ● | ○ | ○ | ● |
| Point mode — arrow keys build a reference | — | ● | ○ | ○ | ○ |
| Friendly formulas — `Sum(Number: B2:B7)` | ● | ● | ○ | ○ | ● |
| Explain a nested formula, unfolded | ● | ● | ○ | ○ | ● |
| The 110 functions as a browsable list | ● | ○ | ○ | ○ | ● |
| Read a formula through the document's names | ● | ● | ● | ○ | ○ |
| Every calculated cell, searchable | ● | ● | ○ | ○ | ○ |
| Find / replace over cells | ○ | ○ | ○ | ○ | ○ |

ᵃ The TUI edits on a formula line rather than in the cell; the browser edits in the formula bar
only. Both are `App::enter` underneath, so the *rule* is identical and only the surface differs.
ᵇ A vi register, not the system clipboard — a terminal cannot reach one without a protocol the
host may not speak (`doc/tui-shell.md`).
ᶜ `grind sheet functions --long` is the same four columns the Win32 dialog and the GNOME
window's autocomplete read from; a completion popup is not a thing a pipe has.
ᵈ As the live result of the formula being typed, before it is committed.
ᵉ `grind sheet eval` over the range. Both status bars generate the three formulas and ask
`App::preview`, rather than keeping a second summing loop.

## 5. Spreadsheet — formatting

The single largest divergence in the suite is this table's last column.

| | CLI | Sheet GTK | TUI | Web | Win32 |
|---|---|---|---|---|---|
| Bold, italic | ● | ● | ● | ● | ○ |
| Alignment | ● | ● | ● | ● | ○ |
| Wrap text | ● | ● | ● | ● | ○ |
| Borders | ● | ○ ᵃ | ● | ● | ○ |
| Text colour, cell background | ● | ● | ● | ● | ○ |
| Clear formatting | ● | ● | ● | ● | ○ |
| Number formats — the eight presets | ● | ● | ● | ● | ○ |
| Decimal places, grouping, currency symbol | ● | ● | ◐ ᵇ | ◐ ᶜ | ○ |
| Read a cell's style / format back | ● | ● | ● | ● | ○ |
| **Drawn**: bold, italic, alignment | — | ● | ● | ● | ● |
| **Drawn**: colours | — | ● | ◐ ᵈ | ● | ● |
| **Drawn**: borders | — | ◐ ᵉ | ○ | ● | ○ |
| **Drawn**: wrapped text | — | ● | ○ | ● | ○ |

ᵃ **A real gap, and this table is where it became visible.** Nothing in `ui_sheet_gtk` writes
`CellStyle::borders`; the strip carries bold, italic, three alignments, wrap, two colour
buttons, Clear Formatting and the number-format menu, and no border control. The window *draws*
borders. `grind sheet style --border` and the browser's two palette verbs both set them.
ᵇ `:format number [n]` takes a decimal count; the symbol and the locale are `grind sheet format`'s.
ᶜ More / fewer decimals only.
ᵈ Sixteen terminal colours, nearest match — the medium, not a gap (`doc/tui-shell.md`).
ᵉ A border's *line style* is ignored, so `dashed` and `double` draw solid.

## 6. Spreadsheet — structure, charts and interchange

| | CLI | Sheet GTK | TUI | Web | Win32 |
|---|---|---|---|---|---|
| Add / rename / delete a sheet | ● | ● | ● | ● | ● |
| Rename carries every reference with it (D10) | ● | ● | ● | ● | ● |
| Set a column width or row height | ● | ● | ○ | ○ | ○ |
| Drag a track edge to resize | — | ● | ○ | ○ | ○ |
| Autofit a column | ● | ● | ○ | ○ | ○ |
| Row auto-height from content (L3) | — | ● | ○ | ○ | ○ |
| **Honours** the document's widths and heights | ● | ● | ○ ᵃ | ● | ● |
| Hide / unhide a row or column | ● | ● | ○ | ○ | ○ |
| **Honours** hidden tracks | ● | ● | ● | ● | ● |
| Create or clear a filter | ● | ● | ○ | ○ | ○ |
| **Honours** a filter | ● | ● | ● | ● | ● |
| Define, redefine or delete a name | ● | ● | ○ | ○ | ○ |
| Sees the document's defined names | ● | ● ᶜ | ● ᵈ | ● ᵉ | ● ᶠ |
| Add, edit, remove, move or restyle a chart | ● | ● | ○ | ○ | ○ |
| **Draws** a chart | — | ● | ○ ᵇ | ● | ○ ᵇ |
| Import CSV / TSV | ● | ○ | ○ | ○ | ○ |
| Export CSV / TSV | ● | ○ | ○ | ○ | ○ |
| Cell roles overlay (V6) | ● | ● | ● | ● | ● |
| Name-anchor overlay (V4) | ● | ● | ● | ● | ● |
| Every cell's formula at once | ● ᵍ | ○ | ○ | ○ | ○ |

ᵃ Every column is ten cells wide; the widths are read and written back untouched.
ᵇ Read, kept and written back untouched — a deliberate stop, not a stub.
ᶜ A **Names…** dialog, and the name box resolves one — though only within the open sheet, since
"navigating to another sheet is the sheet tabs' job".
ᵈ The `:names` overlay, and the name plus the formula read through its names on the formula line.
`:<address>` is `a1::parse` only, so it does **not** jump to a name.
ᵉ The palette offers each name as a place to go.
ᶠ The name box offers them, and the overlay draws each anchor.
ᵍ `grind sheet view --formulas`. Every GUI shows the *active* cell's formula in a formula bar or
on a status line; none has a whole-grid "show formulas" mode.

## 7. Word processor

| | CLI | Text GTK | TUI | Web | Win32 |
|---|---|---|---|---|---|
| **Caret and selection** | | | | | |
| Motion by character, line, page, document | ● | ● | ● | ● | ● |
| Home / End on the **wrapped** line | ● | ● | ● | ● | ● |
| Selection by Shift+arrow | — | ● | ● ᵃ | ● | ● |
| Selection by click, drag, Shift+click | — | ● | — | ● | ● |
| **Editing** | | | | | |
| Type, Enter, Backspace, Delete | ● | ● | ● | ● | ● |
| Markdown as you type (`**bold**`, `# `, ``` ``` ```) | ● | ● | ● | ● | ● |
| Insert / delete / move whole blocks by address | ● | ○ | ◐ ᵇ | ○ | ○ |
| System clipboard | — | ○ ᶜ | ◐ ᵈ | ● | ● |
| Find | ● | ○ | ● | ○ | ○ |
| Replace | ● | ○ | ● | ○ | ○ |
| Word count | ● | ● | ● | ● | ● |
| **Character formatting** | | | | | |
| Bold, italic, underline | ● | ● | ● | ● | ● |
| Strikethrough | ● | ● | ● | ● | ◐ ᵉ |
| Monospace / code | ● | ◐ ᵉ | ● | ◐ ᵉ | ◐ ᵉ |
| Colour, highlight | ● | ○ | ● | ● | ○ |
| Font family, font size | ● | ○ | ○ | ○ | ○ |
| Clear formatting | ● | ◐ ᶠ | ● | ● | ○ |
| **Drawn**: the four booleans | — | ● | ● | ● | ● |
| **Drawn**: colour, highlight | — | ○ ᵍ | ◐ ʰ | ● | ○ |
| **Drawn**: family, size | — | ◐ ⁱ | ○ ʰ | ● | ◐ ⁱ |
| **Block structure** | | | | | |
| Paragraph, Heading 1–3 | ● | ● | ● | ● | ● |
| Heading 4–6 | ● | ○ | ● | ◐ ʲ | ● |
| Title, Subtitle | ● | ○ | ● | ● | ○ |
| List item | ● | ○ | ● | ● | ● |
| Change a list item's depth | ● | ○ | ● ᵐ | ● ⁿ | ● ᵒ |
| A named paragraph style | ● | ○ | ● | ○ | ○ |
| **Addressing and navigation** | | | | | |
| `p12`, `p12+40`, `#bookmark`, `§2.1.3` | ● | ● | ● | ● | ● |
| Outline, each row a jump | ● | ● | ◐ ᵏ | ● | ● |
| Create a bookmark | ● | ○ | ○ | ○ | ○ |
| Show where bookmarks anchor (V7) | ● | ● | ◐ ˡ | ◐ ˡ | ◐ ˡ |
| **Pictures** | | | | | |
| Insert an image | ● | ○ | ○ | ○ | ○ |
| **Draws** an image | — | ● | ○ | ● | ○ |

ᵃ Visual mode (`v`), which is the same anchor-plus-caret model under vi's spelling.
ᵇ `o` opens a paragraph below and `X` deletes the block; there is no move.
ᶜ **The one shell with neither a clipboard nor a register.** `App::erase` takes two carets and
the selection can name them; nothing puts either end on a `gdk::Clipboard`.
ᵈ A vi register, plain text, a newline splitting a block.
ᵉ Reachable only by typing the markdown notation (`~~struck~~`, `` `code` ``) — no button, no
key, no menu item. On Win32 the toggle exists in `text_emphasise` and nothing calls it with
`Emphasis::Strike` or `::Code`.
ᶠ Toggling each of the four off; there is no one-shot Clear.
ᵍ **Named**: `run_attributes` emits the family and the four booleans, and each line is then
painted in one theme ink, so a document that coloured a word draws it in the theme's foreground.
ʰ Sixteen colours, nearest match; one font at one size, so a code run is *dimmed* instead.
ⁱ The family is honoured, the size deliberately is not — a line's height is one `line_height`
per fragment, so honouring a size in the width and not in the height would measure a big word
wide on a line too short to hold it.
ʲ Heading 4 only.
ᵏ Printed to the status line rather than opened as a pane.
ˡ The name is drawn at the end of the line the anchor falls on rather than at its offset in it.
Only `grind-text-gtk` puts a tick at the exact offset — it already has `x_at` for the caret.
ᵐ `:li [depth]`. ⁿ Tab and Shift+Tab. ᵒ Ctrl+Shift+K's block-kind dialog lists four depths;
there is no Tab.

## 8. The divergences that matter, ranked

Everything here is reachable from the CLI, which is R9 doing its job. The order is by how much
of a client's own job is missing.

1. **`grind-win32` cannot format a cell at all.** Not bold, not an alignment, not a colour, not
   a number format — `ui_win32` calls neither `App::set_style` nor `App::set_format`. It
   *draws* alignment, bold, italic, text colour and background, so a document formatted
   elsewhere looks right and nothing in this window can produce one. `doc/windows-shell.md`
   decision 4 anticipates exactly this — "a property of the selection goes on the format strip
   (which is W5's, since `CharStyle` **and** `CellStyle` bound it)" — and W5b built the
   `CharStyle` half only. No milestone claims the other half and no line of "What it will not
   do" names it, so it is currently a gap by omission rather than by decision: the single
   largest hole in this matrix, and the only one that was invisible in all five shell documents.
2. **`grind-sheet-gtk` has no borders control** (§5 ᵃ). The one formatting property the most
   complete spreadsheet shell cannot write, and both the browser and the terminal can.
3. **`grind-text-gtk` has no clipboard.** The only shell in the suite, either document type,
   with neither a system clipboard nor a register.
4. **`grind-text-gtk` cannot author a list, a `Title`/`Subtitle`, or a heading past level 3** —
   it draws all of them. Every other client can make at least some of them, and the Win32 pane
   can make all of them from one dialog.
5. **No browser client has formula assist.** Autocomplete, signature hints and point mode are
   `doc/web-shell.md`'s own "largest remaining gap", and the GNOME and Windows shells both have
   the first two out of one shared `grind_sheet::formula::assist`.
6. **CSV is CLI-only.** `import-csv` and `export-csv` appear in no shell. This is the widest
   *row* in the matrix — five ○ against one ● — and the only interchange format the suite has
   besides ODF.
7. **Column widths, row heights, hidden tracks, filters and defined names are CLI-and-GNOME
   only.** Every other client honours all five faithfully and can create none of them.
8. **Charts are CLI-and-GNOME to author, and only the browser joins them in drawing one.**
9. **An image can only be inserted from the CLI**, and only two clients draw one.
10. **Find and replace exist for text on the CLI and in the terminal, and nowhere else** — and
    for *cells* they exist nowhere at all.
11. **Text undo is not on the CLI** (§2 ᶜ) — the one row where a shell is ahead of the CLI, and
    it is a decision about `grind_text::Action` rather than about the CLI.
12. **`grind-sheet-gtk` has no cross-app handoff** (§2 ᶠ), where its twin does.

## 9. Absent from every client

Not a parity problem — a feature line. Each has its row in `doc/not-doing.md` or a gate in a
shell document, and none of them is reachable from the CLI either.

**Spreadsheet.** Conditional formatting · merged-cell rendering (the model carries no spans) ·
freeze panes · sort · find/replace over cells · autosave · printing · pivot tables · macros
(`doc/not-doing.md` §1 — the generator is the answer, and `grind build` is a CLI verb by R11).

**Word processor.** Tables · footnotes · fields (`text:page-number`, `text:date`, …) · style
*definitions* (a named character style is kept and never interpreted) · pages · printing · an
image anchored mid-sentence, which draws as the placeholder character everywhere.

**Both.** Pagination and RTL, both gated — RTL by explicit decision in `doc/text-layout.md`.
Editing the code view (`doc/dsl.md` §6.4). `grind test`, D8's half of the generator.

## 10. How to re-derive this

The tables were read out of the code, not out of the shell documents, in this order:

```sh
# 1. What each application can do at all — the two authorities on the rows.
grep -oE "^\s*pub fn [a-z_0-9]+" sheet/src/lib.rs text/src/lib.rs

# 2. What each client reaches. Every shell has one table of verbs, and this is where it is:
sed -n '/pub const SHEET/,/^];/p' ui_web/src/command.rs   # and TEXT, below it
sed -n '/pub const MENUS/,/^];/p' ui_win32/src/menu.rs    # plus `accelerator` and `applies_to`
sed -n '/fn actions/,/^}/p'      ui_sheet_gtk/src/main.rs
sed -n '/fn actions/,/^}/p'      ui_text_gtk/src/main.rs
grep -n 'pub const HELP'         ui_tui/src/sheet/mod.rs ui_tui/src/text/mod.rs
grind sheet --help; grind text --help

# 3. The cross-check, which is what actually caught §8's first two rows: a verb table can
#    promise anything, and a shell that never calls the core method cannot deliver it.
for d in ui_sheet_gtk ui_text_gtk ui_tui ui_web ui_win32; do
  echo "== $d"; grep -rhoE '\.(set_style|set_format|set_filter|set_col_width|import_csv)\(' $d/src | sort -u
done
```

Step 3 is the one worth keeping. A gap list is written by the person who built the shell, and
what they leave out is exactly what they did not notice they had left out — `ui_win32` calling
neither `set_style` nor `set_format` is four seconds of `grep` and was in none of the five
documents that describe these clients.

**What would make this file self-checking**, if it earns it: the same shape everything else here
uses — a test that reads each shell's own verb table and this document's tables and fails when
they disagree, the way `cli/tests/parity.rs` reads `sheet/src/lib.rs`. It is not built. The
honest reason is that a shell's verb table is five different shapes (a `const` slice, a `Vec` of
tuples, a `&str` of help text, an HTML file, a `match`), and normalising all five is a bigger
piece of work than the one thing it would buy: this file going stale is visible the first time
somebody reads it beside a shell, where a broken ratchet is visible immediately. Until then it
carries a date.

**Read on 2026-09-08**, at `main`. The clients as of then: `grind sheet` complete through phase
9, `grind text` through S10, the GNOME spreadsheet through M10 plus charts, filters and the
chrome rework, the Windows shell through W9.
