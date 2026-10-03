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
other six. Each shell names its own gaps in its own document, which is the right place for the
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
| **Text GTK** | `grind-text-gtk` | `grind-text-gtk` | text only | header bar, format bar (paragraph style first), primary menu, a context menu on the page |
| **TUI** | `grind-tui` | `grind-tui` | both, one binary | vi keys and a `:` command line |
| **Web** | `grind-web` | one wasm bundle | both, one bundle | verb bar, one tool row per type, **Ctrl+K palette** |
| **Win32** | `grind-win32` | `grind-win32.exe` | both, one binary | **menu bar**, context menus, format strip |
| **Mac** | `grind-mac` | `Grind.app` | both, one binary | **menu bar**, toolbar, sidebar, context menus |

The GTK pair is two binaries **by decision** (`doc/suite.md`, "One binary per document type —
for GTK only"): a `.desktop` file's `MimeType=` is per application. Everywhere else one binary
opens either kind, dispatched on `grind_core::kind` from the file's bytes.

**Legend.** ● built · ◐ partial, see the notes under the table · ○ absent, and deferred by
decision · — not applicable to this client.

**The Mac column means *written*, not *seen working*.** It is read out of `ui_mac` as built and
type-checked from Linux; none of it has run on a Mac yet (`doc/macos-shell.md`, M2–M10), and the
runner's first green `artifacts.yml` job is what turns its ● into the others' ●. Its notes are
numbered, under each table, so they cannot collide with the lettered ones.

## 2. Suite-wide

| | CLI | Sheet GTK | Text GTK | TUI | Web | Win32 | Mac |
|---|---|---|---|---|---|---|---|
| Opens a spreadsheet | ● | ● | — | ● | ● | ● | ● |
| Opens a text document | ● | — | ● | ● | ● | ● | ● |
| Reads all three forms (`.ods`/`.fods`, `.odt`/`.fodt`, `.grind`) | ● | ● | ● | ● | ● | ● | ● |
| Writes all three forms | ● | ● | ● | ● | ● | ● | ● |
| Flat-first default (`doc/flat-first.md`) | ● | ● | ● | ● | ● | ● | ● ¹ |
| New, empty document | ● | ● | ● | ● ᵗⁿ | ● | ● | ● |
| A **welcome screen** with no document open | — | ○ | ○ | ○ | ● | ● | ● |
| New document of the *other* kind, in place | — ⁱ | ○ | ○ | ● ᵗⁿ | ● | ● | ● ² |
| Undo / redo — spreadsheet | ◐ ᵇ | ● | — | ● | ● | ● | ● |
| Undo / redo — text | ○ ᶜ | — | ● | ● | ● | ● | ● |
| Code view — the projection, read-only (D9) | ● | ● | ● | ● | ● | ● | ● |
| Check Document — `lint` findings, each a jump (D6) | ● | ● | ● | ● | ● | ● | ● ³ |
| Go to an address | ● | ● | ● | ● | ● | ● | ● ⁴ |
| Key list / help | ● | ● | ● | ● | ◐ ᵈ | ● | ● ⁵ |
| About / build stamp | ● | ● | ● | ● ʰ | ● ʷᵃ | ● | ● ⁶ |
| Recent files | — | ● | ● | ○ | ○ | ● ʷʳ | ● |
| Opening the *other* document kind | ● ᵉ | ○ ᶠ | ● ᵉ | ● | ● | ● | ● |
| Assertable headless output | stdout | `--render-to` PNG | `--render-to` PNG | `TestBackend` | `smoke.js` (jsdom) | `--render-to` BMP | `--render-to` PNG, `--drive` |
| `.deb` + `.rpm` | ● | ● | ● | ● | — | — | — ⁷ |
| Accessibility floor | — ᵍ | announce | announce | terminal | ARIA labels | system caret | text area, announce |

ᵗⁿ `:new [sheet|text]` and `:open <file>` (`!` to discard unsaved work) replace the pane in the same terminal; the event loop swaps it (`app::Switch`) — checked in a pty, 2026-10-02.
ᵃ `grind-tui --sheet` / `--text` starts an empty document; there is no in-session "new".
ⁱ `grind sheet new` / `grind text new` make one, which is the same capability; "in place" is a
question only a window with one document open at a time has. The two GTK shells are one document
type each (`doc/suite.md`), so the row means nothing there; `grind-tui` has no "new" at all — see
ᵃ — which makes it the one client where the welcome screens' verbs have no equivalent.
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
ʰ `:about`, on the status line — there is no dialog to put it in.
ᵍ A pipe is the accessible surface, which is `doc/view-modes.md` §4.6's argument for why the
CLI matters most exactly where a GUI's whole output is colour.

ʷʳ Every document opened or saved is handed to `SHAddToRecentDocs`, so it is in the taskbar jump list and every file dialog's Recent — the platform's own list, with none of ours (2026-10-02; build-checked only).
ʷᵃ *About this build* in the palette says `grind_core::build_info::describe` on the message line — no dialog to put it in (2026-10-02).

**Mac.** ¹ An untitled document saves flat: the document answers `fods` or `fodt` itself (M10),
since the type it is declared under would have said `.ods`. ² A window of its own beside this
one — one window per document is decision 5. ³ The sidebar's Problems section rather than a
dialog, every row a jump (M8). ⁴ The grid's name box (⌘L) takes any address; on a page ⌘L
asks for one in an alert, and its sidebar's outline and bookmarks are jumps too. ⁵ Every menu item shows
its key, and Help ▸ Search finds any item — the platform's own key list. ⁶ The standard About
panel, told `grind_core::build_info`'s stamp: the version, the commit and the build, and when. ⁷ A universal,
ad-hoc-signed `Grind.app` in a DMG instead (M10).

## 3. Spreadsheet — navigation and selection

| | CLI | Sheet GTK | TUI | Web | Win32 | Mac |
|---|---|---|---|---|---|---|
| Move by cell, row, page, document | — | ● | ● | ● | ● | ● |
| Jump to the edge of a block (Ctrl+arrow) | — | ● | ● ᵉᵈ | ● ᵉᵈ | ● | ● |
| Jump to the start or end of the sheet | — | ● | ● | ● | ● | ● |
| Select a rectangle | ● ᵃ | ● | ● | ● | ● | ● |
| Select whole rows / columns from a header | ● ᵃ | ● | ● ᵉᵈ | ○ | ● | ● |
| Select the whole sheet | ● ᵃ | ● | ● ᵉᵈ | ● | ● | ● |
| Name box / address field | ● | ● | ● | ● | ● | ● |
| Go to a defined name | ● | ● | ● | ● | ● | ● |
| Skip a hidden or filtered row while moving | — | ● ʷᵉ | ● ᶜ | ● ʷᵉ | ● | ● |
| Sheet switching | address | tab strip | `:sheet` | tab strip | tab strip + Ctrl+PgUp/PgDn | sidebar |
| Zoom | — | ● | ○ | ○ | ○ | ● |

ᵉᵈ Closed 2026-10-02 over `grind_sheet::nav`'s edge rule: the browser's Ctrl+arrow (Shift extends); the terminal's `w b } {` and Ctrl+arrows, with `V` (row), Ctrl+V (column) and Ctrl+A (sheet) as visual selections of the used part — no header to click in a terminal.
ᵃ A range is an argument, not a gesture: `A1:C9`, `A:A`, a sheet-qualified form.
ʷᵉ The GNOME grid applies `nav::onto_visible` after its move (closing the gap `doc/sheet-shell.md` named; lint-checked only). The browser's motions are `grind_sheet::nav::moved` now, then `nav::onto_visible` over the hidden and filtered tracks — the rule the Windows and Mac grids use (2026-10-02). It replaces the browser's own `keymap::moved`, so selection and Shift-extend behave as they do there.
ᵇ **Named** in `doc/sheet-shell.md`: `keymap.rs` is pure and knows nothing about the document,
so skipping means handing it the hidden set.
ᶜ Every motion counts in tracks that are **drawn** (`ui_tui/src/sheet/keymap.rs`'s `walk`), on
both axes and including a page. It was a bug until it was a feature: stepping onto a folded row
put the cursor where nothing was on screen, which reads as a terminal dropping keystrokes rather
than as a cursor on a hidden row.

## 4. Spreadsheet — editing and formulas

| | CLI | Sheet GTK | TUI | Web | Win32 | Mac |
|---|---|---|---|---|---|---|
| The typing rule (`=` formula, `'` text, empty clears) | ● | ● | ● | ● | ● | ● |
| In-cell editor | — | ● | ○ ᵃ | ○ ᵃ | ● | ● |
| Formula bar | — | ● | ◐ ᵃ | ● | ● | ● ¹ |
| Clear a cell or a range | ● | ● | ● | ● | ● | ● |
| Clear only the formula, keeping the value | ● | ○ | ● ᵗᵘ | ● | ● | ● |
| Paste a rectangle of tab-separated rows | ● | ● | ● | ● | ● | ● |
| System clipboard (cut / copy / paste) | — | ● | ◐ ᵇ | ● | ● | ● |
| Copy Value — the formatted result, not the formula | ● | ● | ● ᵗᵘ | ● | ● | ● |
| Fill down / fill right | ● | ● | ● | ● | ● | ● |
| Fill one cell across a rectangle (references shifted) | ● | ● | ● | ● | ● | ● |
| Recalculate | ● | ● | ● | ● | ● | ● ² |
| Stale-value warning | ● | ● | ● | ● | ● | ● |
| Evaluate a formula without storing it | ● | ● ᵈ | ● | ● ᵖᵃ | ● | ● |
| Selection arithmetic — Sum, Count, Average | ● ᵉ | ● | ● ᵉ | ● | ● | ● |
| Autocomplete while typing a formula | ● ᶜ | ● | ● | ○ | ● | ● |
| Signature hint for the call the caret is in | ● ᶜ | ● | ● | ○ | ● | ● |
| Point mode — arrow keys build a reference | — | ● | ● ᵖᵐ | ● ᵖᵐ | ● ᵖᵐ | ● |
| Friendly formulas — `Sum(Number: B2:B7)` | ● | ● | ◐ ᵉˣ | ◐ ᵉˣ | ● | ● |
| Explain a nested formula, unfolded | ● | ● | ◐ ᵉˣ | ◐ ᵉˣ | ● | ● |
| The 110 functions as a browsable list | ● | ● ᶠᵍ | ● ᶠᵍ | ● ᶠⁿ | ● | ● |
| Read a formula through the document's names | ● | ● | ● | ○ | ○ | ● |
| Every calculated cell, searchable | ● | ● | ● ᶜᵃ | ◐ ᶜᵃ | ● ᶜᵃ | ● |
| Find over cells | ● | ● ᵍ | ● ᶠ | ● ʰ | ● ⁱ | ● |
| Replace over cells | ● | ● ᵍ | ● ᶠ | ● ʰ | ● ⁱ | ● |

ᵖᵐ Where `ref_eligible` says a reference could go, the four arrows point (one cell, no range extension) and write the address into the edit line; the pointed cell is drawn in magenta. Closed 2026-10-02; the browser does the same in the formula bar (UTF-16 offsets through `grind_core::utf16`), writing the address without highlighting the cell — compile- and lint-checked only. Win32 does it in the in-cell `EDIT` through `EM_REPLACESEL` (Enter mode starts one; Edit mode keeps the caret) — also checked only by build.
ᶠⁿ Type two letters of a function's name or plain-English name in the palette: up to five rows follow the verbs, and picking one starts an edit seeded `=NAME(`. A search, not a scrolling list — 2026-10-02.
ᶠᵍ *Functions…* in the GNOME palette (a searchable dialog) and `:functions [text]` in the terminal (a list pane): name, plain-English name, brief and category from `funcs::catalog`; picking one starts an edit seeded `=NAME(`. 2026-10-02.
ᶜᵃ `:calc [text]` (a list pane) and Data ▸ Find a Calculation… (a prompt, then a list) over `App::calculations`; each row a jump. The browser's palette find already matches a cell's formula text, which is the half of it. 2026-10-02.
ᶠᵒ `:formulas` in the terminal, a palette verb in the browser, View ▸ Show Formulas in Windows and GNOME (the GNOME one draws the cell text only, and is lint-checked rather than seen): each formula cell shows its formula text (display syntax) instead of its result — a reading, nothing is written (2026-10-02).
ᵃ The TUI edits on a formula line rather than in the cell; the browser edits in the formula bar
only. Both are `App::enter` underneath, so the *rule* is identical and only the surface differs.
ᵇ A vi register, not the system clipboard — a terminal cannot reach one without a protocol the
host may not speak (`doc/tui-shell.md`).
ᶜ `grind sheet functions --long` is the same four columns the Win32 dialog and the GNOME
window's autocomplete read from; a completion popup is not a thing a pipe has.
ᵈ As the live result of the formula being typed, before it is committed.
ᵉ `grind sheet eval` over the range. All four status bars generate the three formulas and ask
`App::preview`, rather than keeping a second summing loop, and spell the answers through
`App::display_number`, so a German document's sum reads `1234,5` as its cells do. `eval` stays
ISO, as a script reads a number (`doc/cli-parity-sheet.md`).
ᶠ `:find`, then `n`/`N`, with every match marked in the grid, and `:s/old/new/` — the text
half's own spelling — for replace. Both are `App::find`/`App::replace` (`grind_sheet::find`),
which search each cell's *input text* and send a replaced cell back through the typing rule, so
the terminal and `grind sheet find`/`replace` cannot disagree about what a match is.
ᵍ A find bar, Ctrl+F and Ctrl+H (`ui_sheet_gtk/src/find.rs`), over the same two calls —
*Replace* is `App::replace` narrowed to one cell by `Search::range`, which `--in` reaches from
the CLI. It does not mark every hit in the grid, as the terminal does.
ʰ The palette is the find box, per `doc/web-shell.md`'s one design decision: a word lists the
cells holding it after the verbs, F3/Shift+F3 step through them, Ctrl+F opens it, and Ctrl+H
replaces through two prompts. The step is `grind_sheet::find::step`, the GNOME bar's own. No
match-case or whole-cell control.
ⁱ Edit ▸ Find… (Ctrl+F), Find Next/Previous (F3/Shift+F3) and Replace… (Ctrl+H), the questions
asked through `dialog::prompt` and the answers said on the notice bar (`notice::found`,
`notice::replaced`). The same step as the other two. No match-case or whole-cell control.

**Mac.** ¹ A read-out above the grid that reads a formula in plain English at rest, and an editor
of the cell once clicked — the same edit the in-cell editor is. ² Edit ▸ Recalculate,
on ⌘= — Excel for Mac's key — and the banner's Recalculate Anyway.

## 5. Spreadsheet — formatting

This table's last column was the single largest divergence in the suite until W12 gave the
Windows grid its format strip (`ui_win32/src/sheet/format.rs`); what is left of it is wrap and
borders, which that window does not draw either.

| | CLI | Sheet GTK | TUI | Web | Win32 | Mac |
|---|---|---|---|---|---|---|
| Bold, italic | ● | ● | ● | ● | ● | ● |
| Alignment | ● | ● | ● | ● | ● | ● |
| Wrap text | ● | ● | ● | ● | ● ʷʷ | ● |
| Borders | ● | ● ᵃ | ● | ● | ● ʷᵇ | ● |
| Text colour, cell background | ● | ● | ● | ● | ● | ● |
| Clear formatting | ● | ● | ● | ● | ● | ● |
| Number formats — the eight presets | ● | ● | ● | ● | ● | ● |
| Decimal places, grouping, currency symbol | ● | ● | ◐ ᵇ | ◐ ᶜ | ◐ ʷ | ◐ ² |
| Read a cell's style / format back | ● | ● | ● | ● | ● | ● |
| A live sample of a number format before it is set | — | ● ᶠ | ○ | ○ | ○ | ● |
| Set the document's own locale (`doc/ods-format.md` §5.2) | ● | ● ᵍ | ○ | ○ | ○ | ● |
| **Honours** the document's locale — shown and typed | ● | ● | ● | ● | ● | ● |
| **Drawn**: bold, italic, alignment | — | ● | ● | ● | ● | ● |
| **Drawn**: colours | — | ● | ◐ ᵈ | ● | ● | ● |
| **Drawn**: borders | — | ◐ ᵉ | ○ | ● | ◐ ʷᵇ | ◐ ³ |
| **Drawn**: wrapped text | — | ● | ○ | ● | ● ʷʷ | ● ¹ |
| **Drawn**: a number too wide for its column is `###`, never cut (`numfmt::overflow`) | — | ● | ● | ● | ● | ● |

ᵃ **Closed 2026-10-02.** A *Borders* toggle in the format bar writes `format::bordered` — a hairline on every edge, or none — over the selection, the same call the terminal, the browser and the Mac make; a per-edge width or colour is still `grind sheet style --border`'s.
ᵇ `:format number [n]` takes a decimal count and `:format currency [eur|usd|gbp]` one of the three
currencies `numfmt::CURRENCIES` offers; any other symbol and the locale are `grind sheet format`'s.
ᶜ More / fewer decimals, and the three currencies of `numfmt::CURRENCIES` as three menu entries.
ʷ One decimal more or fewer at a time (the strip's `-.0`/`+.0`, and Format ▸ Increase/Decrease
Decimals), and the three currencies of `numfmt::CURRENCIES` — Format ▸ Currency and the cells'
context menu, a currency cell keeping its own decimals and grouping (`ui_win32/src/sheet/currency.rs`).
No grouping toggle and no other symbol.
ʷʷ Closed 2026-10-02: Format ▸ Wrap Text; the grid breaks a wrapped cell with `grind_core::layout::wrap` over GDI measurements (`sheet/measure.rs`) and grows rows through `grind_sheet::autoheight` (hoisted from the Mac) — checked by a Wine render.
ʷᵇ Closed 2026-10-02: Format ▸ All Borders / Remove Borders (`format::bordered`), and the grid draws every edge through `look::border_strokes` — the Mac's geometry, hoisted — checked by a Wine render. `dashed` and `dotted` draw solid.
ˣ Named in `doc/windows-shell.md`'s "What it will not do": this window draws neither wrapped text
nor borders, and a control whose effect cannot be seen is not offered.
ᵈ Sixteen terminal colours, nearest match — the medium, not a gap (`doc/tui-shell.md`).
ᵉ A border's *line style* is ignored, so `dashed` and `double` draw solid.
ᶠ Above the number popover's settings: the active cell as they would show it, from
`App::shown_as`, the renderer's own answer — and the strip's button face says what the cell is
formatted as (`123`, `%`, `€`, `Date`).
ᵍ *Document Settings…*, applied at once as one undo step. **Honouring** a locale costs a client
nothing, which is why that row is full: every cell's text comes out of `App::get_viewport`
already spelled the document's way, and every typed number goes through the core's typing
rule. A new document in the GNOME window states the desktop's locale; `grind sheet new` states
none unless told.

**One rule for a number that does not fit.** Part of a number is a different number —
`2026-08-16` cut to `2026-08-1` is the first of August, `3,710.00 €` cut to `3,710.0…` has lost a
digit and its currency — so a number, date or time too wide for its column is drawn as `###`, as
many whole hashes as the column holds, in every client: `grind_sheet::numfmt::overflow`, measured
in each shell's own unit and decided by the **value** being a number, never by where it is
aligned. Text that does not fit ends in `…`. Before 2026-09-27 the four clients had three answers:
the GNOME window hashed a number only when it was right-aligned, the Windows window ellipsized
every cell, and the browser clipped without a mark — and did not honour a column's width at all
once its content was wider, since a `max-content` table sizes its columns from what is in them.

**Mac.** ¹ Broken at the column's width and drawn as lines, and a row with no height of its own
grown to hold them, as the GNOME window grows one. ² The toolbar's two steps and Format ▸ Number's; no grouping or currency-symbol control. ³ Each edge its own width and colour, and `double` as two lines; `dashed` and `dotted` drawn solid. Format ▸ Borders sets a hairline round every selected cell or takes them all away.

## 6. Spreadsheet — structure, charts and interchange

| | CLI | Sheet GTK | TUI | Web | Win32 | Mac |
|---|---|---|---|---|---|---|
| Add / rename / delete a sheet | ● | ● | ● | ● | ● | ● |
| Rename carries every reference with it (D10) | ● | ● | ● | ● | ● | ● |
| Set a column width or row height | ● | ● | ● | ● ᵖᵃ | ● | ● |
| Drag a track edge to resize | — | ● | ○ | ○ | ○ | ● |
| Autofit a column | ● | ● | ○ | ○ | ● ᶠᶜ | ● |
| Row auto-height from content (L3) | — | ● | ○ | ○ | ● ʷʷ | ● |
| **Honours** the document's widths and heights | ● | ● | ◐ ᵃ | ● | ● | ● |
| Hide / unhide a row or column | ● | ● | ● | ● | ● | ● |
| **Honours** hidden tracks | ● | ● | ● | ● | ● | ● |
| Create or clear a filter | ● | ● | ● ᵗᵘ | ● | ● | ● |
| **Honours** a filter | ● | ● | ● | ● | ● | ● |
| Define, redefine or delete a name | ● | ● | ● | ● ᵖᵃ | ◐ | ● ³ |
| Rename a name, carrying every use (§6.5) | ● | ● ᵖᵇ | ● ᵗᵘ | ● ᵖᵃ | ○ | ● ³ |
| Inline a name into every use (§6.5) | ● | ● ᵖᵇ | ● ᵗᵘ | ● ᵖᵃ | ○ | ● ³ |
| Sees the document's defined names | ● | ● ᶜ | ● ᵈ | ● ᵉ | ● ᶠ | ● ¹ |
| Add, edit, remove, move or restyle a chart | ● | ● | ◐ ᶜʰ | ◐ ᶜʰ | ◐ ᶜʰ | ◐ ⁵ |
| A chart **read from a table** — orientation, labels, kind (`App::suggest_chart`) | ● ⁿ | ● | ● ᶜʰ | ● ᶜʰ | ● ᶜʰ | ● ⁵ |
| A chart's own title and legend | ● | ● | ○ | ● ᶜʰ | ● ᶜʰ | ● ⁵ |
| A **preview** of a chart before it is inserted | — | ● ᵒ | ○ | ○ | ○ | ○ |
| **Draws** a chart | — | ● | ○ ᵇ | ● | ● ᶜʰ | ● |
| **Draws** its title and legend | — | ● | ○ ᵇ | ● | ● ᶜʰ | ● |
| Import CSV / TSV | ● | ● ʰ | ● | ● ʰ | ● ʰ | ● ² |
| Export CSV / TSV | ● | ● ʰ | ● | ● ⁱ | ● ʰ | ● ⁴ |
| An import's seven options (`--text`, `--locale`, …) | ● | ○ ʲ | ○ ʲ | ○ ʲ | ○ ʲ | ○ |
| Open an Excel workbook (phase 11) | ● ᵏ | ● ᵏ | ● ᵏ | ● ᵏ | ● ᵏ | ● |
| Open a CSV / TSV as a document of its own | ● ˡ | ● ˡ | ● ˡ | ● ˡ | ● ˡ | ● |
| Cell roles overlay (V6) | ● | ● | ● | ● | ● | ● |
| Name-anchor overlay (V4) | ● | ● | ● | ● | ● | ● |
| Every cell's formula at once | ● ᵍ | ● ᶠᵒ | ● ᶠᵒ | ● ᶠᵒ | ● ᶠᵒ | ● |

ᵖᵃ Palette verbs, each a `prompt()` over the core's own call — *Row height…*, *Column width…*, *Define a name for the selection…*, *Rename a name…*, *Inline a name into its uses…*, *Delete a name…*, *Evaluate a formula…* — closed 2026-10-02; a typed name, not a picker.
ᵖᵇ Two more buttons on each row of the *Names…* dialog — rename (an alert with an entry) and inline — over `App::rename_name` / `inline_name`, each saying how many uses it rewrote; closed 2026-10-02, compile- and lint-checked only.
ᵗᵘ Terminal verbs, closed 2026-10-02: `:filter` (no dropdowns drawn), `:rename`, `:inline`, `:value`, `:yank-values`, `:across`.
ᵉˣ `:explain` and the palette's *Explain this formula in words* say the active cell's formula on one line (`friendly::explain_inline`) — not unfolded, and the formula bar does not read friendly at rest. Closed in part 2026-10-02.
ᶠᶜ Sheet ▸ Fit Column Width, measured in the cell font by GDI; from the menu, not a double-click on the edge — 2026-10-02.
ᵃ The **widths** are honoured, in whole terminal cells (`ui_tui/src/sheet/geom.rs`); a row is
one line of a terminal, so a **height** is stored and not drawn and `:height` says so on the
status line. Both are written back untouched either way.
ᵇ Read, kept and written back untouched — a deliberate stop, not a stub.
ᶜ A **Names…** dialog, and the name box resolves one — though only within the open sheet, since
"navigating to another sheet is the sheet tabs' job".
ᵈ The `:names` overlay, the name plus the formula read through its names on the formula line,
and `:<address>` resolves a **name before an address** — it has to, since `tax_rate` parses
perfectly well as a cell reference and naming a range would otherwise make it unreachable.
ᵉ The palette offers each name as a place to go.
ᶠ The name box offers them, and the overlay draws each anchor.
ᵍ `grind sheet view --formulas`. Every GUI shows the *active* cell's formula in a formula bar or
on a status line; none has a whole-grid "show formulas" mode.
ʰ **Zero-prompt, and the same behaviour in all four shells** — a file picker and nothing else.
The delimiter is read out of the file's own content (`csv::Dialect::sniff`), the fields land at
the **cursor** rather than at A1, and an export writes the selection, or everything the sheet
uses when the selection is one cell. Which delimiter goes *out* is the name's to say
(`csv::Dialect::for_name`: `.tsv` is tabs), which is what a save dialog's two filters are for.
ⁱ A browser download names itself, so the choice that is a save-dialog filter elsewhere is two
palette rows here: *Export as CSV* and *Export as TSV*.
ʲ **Named, and it is the same gap in all four**: a window imports with `csv::Import::sniffed` —
the sniffed delimiter plus `dates`, which is ISO-only and so cannot misread a field — and has no
dialog for `--text`, `--formulas`, `--locale`, `--trim` or a delimiter the sniffer got wrong.
`grind sheet import-csv` has all seven (R9), and `doc/not-doing.md`'s "a CSV column typed by
hand" is the row this sits under.
ᵏ `grind sheet import`, and in every window the ordinary Open (X6, `doc/xlsx-import.md`): sniffed
from the bytes, imported through `grind_xlsx::open`, and opened as a **new, unsaved ODF
document with no path** under `grind_xlsx::suggested_name` (`budget.xlsx` → `budget.fods`), so
Save asks where to put it and nothing can write ODF over the workbook. Every shell shows the
same `Report::summary` sentence — a toast, the status line, the page's message, the notice bar.
The word processor's GTK window hands a workbook to the spreadsheet one, as it does an `.ods`.
Behind each crate's `xlsx` feature, on by default.
ˡ The same shape as ᵏ, through `grind_sheet::csv::open`: a `.csv`, `.tsv` or `.tab` whose bytes
are neither ODF nor a workbook — plain text has no signature, so the name is asked last — is read
the way ʲ's *Import CSV* reads one, into `A1` of a sheet named after the file, and opened as a new,
unsaved `data.fods` with no path and a one-sentence summary. Every window's Open reaches it, and
so does a double-click (`doc/suite.md`, "Mime types"; `doc/windows-shell.md`, "File
associations"). The CLI's spelling is two verbs, `sheet new` and `sheet import-csv`, which is
what the function does.
ⁿ `grind sheet chart-add --from A1:C7`, with any flag overriding a piece of what it read.
ᵒ The chart dialog's top third is the chart, drawn by the grid's own painter from
`App::preview_chart` — `add_chart`/`edit_chart` without the write — and a new chart lands beside
the table it was made from (`doc/chart-format.md`, The shell).

**Mac.** ¹ In the sidebar's Names section, and the name box takes one. ² A CSV opens as a new,
untitled spreadsheet of its own (the next row), and File ▸ Import CSV… reads one into the open
sheet at the active cell, sniffed.
³ Insert ▸ Name… defines one over the selection; a name's row in the sidebar has a context
menu that renames it with every use following, redefines it, inlines it everywhere or deletes it.
⁴ File ▸ Export as CSV…, the sheet's used rectangle with shown values, its delimiter picked by
the name — `.tsv` is tabs. ⁵ Insert ▸ Chart charts the table the selection is in, beside it,
as the core reads it; a right-click on a chart sets its title, its kind and its legend or removes
it, and dragging one moves it. There is no dialog, and resizing or recolouring one is the CLI's.

## 7. Word processor

| | CLI | Text GTK | TUI | Web | Win32 | Mac |
|---|---|---|---|---|---|---|
| **Caret and selection** | | | | | | |
| Motion by character, line, page, document | ● | ● | ● | ● | ● | ● |
| Home / End on the **wrapped** line | ● | ● | ● | ● | ● | ● |
| Selection by Shift+arrow | — | ● | ● ᵃ | ● | ● | ● |
| Selection by click, drag, Shift+click | — | ● | — | ● | ● | ● |
| **Editing** | | | | | | |
| Type, Enter, Backspace, Delete | ● | ● | ● | ● | ● | ● |
| Markdown as you type (`**bold**`, `# `, ``` ``` ```) | ● | ● | ● | ● | ● | ● |
| Insert / delete / move whole blocks by address | ● | ◐ ᵍᵖ | ● ᵇ | ◐ ᵍᵖ | ◐ ᵍᵖ | ◐ ³ |
| System clipboard | — | ● ᶜ | ◐ ᵈ | ● | ● | ● |
| Find | ● | ● | ● ᶠ | ● ˣ | ● ʷ | ● |
| Replace | ● | ● ʸ | ● | ● ˣ | ● ʷ | ● |
| Word count | ● | ● | ● | ● | ● | ● |
| **Character formatting** | | | | | | |
| Bold, italic, underline | ● | ● | ● | ● | ● | ● |
| Strikethrough | ● | ● | ● | ● | ● | ● |
| Monospace / code | ● | ● | ● | ◐ ᵉ | ● | ● |
| Colour, highlight | ● | ● | ● | ● | ● | ● |
| Font family, font size | ● | ● | ○ | ○ | ● | ● ¹ |
| Clear formatting | ● | ● | ● | ● | ● | ● |
| **Drawn**: the four booleans | — | ● | ● | ● | ● | ● |
| **Drawn**: colour, highlight | — | ● | ◐ ʰ | ● | ● | ● |
| **Drawn**: family, size | — | ● ⁱ | ○ ʰ | ● | ◐ ʳ | ● |
| **Block structure** | | | | | | |
| Paragraph, Heading 1–3 | ● | ● | ● | ● | ● | ● |
| Heading 4–6 | ● | ● | ● | ◐ ʲ | ● | ● ² |
| Title, Subtitle | ● | ● | ● | ● | ● | ● ² |
| List item | ● | ● | ● | ● | ● | ● |
| Change a list item's depth | ● | ● ⁿ | ● ᵐ | ● ⁿ | ● ᵒ | ● |
| A named paragraph style | ● | ● ᵍᵖ | ● | ● ᶻ | ● ᶻ | ● |
| **Addressing and navigation** | | | | | | |
| `p12`, `p12+40`, `#bookmark`, `§2.1.3` | ● | ● | ● | ● | ● | ● ³ |
| Outline, each row a jump | ● | ● | ● | ● | ● | ● ⁴ |
| Create a bookmark | ● | ● ᵍᵖ | ● | ● ᶻ | ● ᶻ | ● |
| Show where bookmarks anchor (V7) | ● | ● | ◐ ˡ | ◐ ˡ | ◐ ˡ | ● |
| **Pictures** | | | | | | |
| Insert an image | ● | ● | ● ᵗⁱ | ● ᵖⁱ | ● | ● |
| **Draws** an image | — | ● | ○ | ● | ● | ● |
| **Tables** | | | | | | |
| Insert a table | ● | ● | ● | ● ᶻ | ● ᶻ | ● |
| Edit inside a cell | ● | ● | ● ᵖ | ● ᵖ | ● ᵖ | ● |
| **Draws** a table as a grid | — | ● | ● ᵠ | ● | ● ᵖ | ● |
| Merge cells, set a column width | ○ ᵗ | ○ ᵗ | ○ ᵗ | ○ ᵗ | ○ ᵗ | ○ |

ᵃ Visual mode (`v`), which is the same anchor-plus-caret model under vi's spelling.
ᵇ `o` opens a paragraph below, `X` deletes the block, `:move <address>` puts it elsewhere.
ᶜ Cut/Copy/Paste over `gdk::Clipboard`, plain text both ways, a newline being a block boundary —
the same two halves `grind-web` has. This used to be the one shell in the suite with neither a
clipboard nor a register.
ᵈ A vi register, plain text, a newline splitting a block.
ᵉ Reachable only by typing the markdown notation (`~~struck~~`, `` `code` ``) — no button, no
key, no menu item. On Win32 the toggle exists in `text_emphasise` and nothing calls it with
`Emphasis::Strike` or `::Code`. `grind-text-gtk`'s format bar has a Monospace toggle, which is
the same thing under the name the document uses: `` `code` `` is a *family*, not a fifth boolean.
ʰ Sixteen colours, chosen to read on a light terminal and a dark one (`ui_tui/src/ink.rs`); one font at one size, so a code run is *dimmed* instead.
ⁱ Both, and the size reaches the line's *height* as well as its width — `metrics::size_units` is
one parse feeding the measuring attribute, `Metrics::line_height` and the drawing attribute, so a
24pt word makes room for itself. A size in a unit with no resolution here (`5cm`) is left alone.
ʳ The family is honoured, the size deliberately is not — a line's height is one `line_height`
per fragment, so honouring a size in the width and not in the height would measure a big word
wide on a line too short to hold it.
ʲ Heading 4 only.
ˡ The name is drawn at the end of the line the anchor falls on rather than at its offset in it.
Only `grind-text-gtk` puts a tick at the exact offset — it already has `x_at` for the caret.
ᵐ `:li [depth]`. ⁿ Tab and Shift+Tab — in `grind-text-gtk`, Tab at the front of a paragraph
starts a list and Shift+Tab out of depth 1 ends it. ᵒ Ctrl+Shift+K's block-kind dialog lists four
depths; there is no Tab.
ᵖ **Free, and that is the point of the model.** A cell holds *blocks* and a block carries the
coordinate of the cell it is in (`grind_text::Cell`), so `p12` is the twelfth block whether it is
in a table or not — every caret motion, every formatting edit and every address already worked
inside a cell before any client knew tables existed. **Every client now also draws the grid.**
`grind-web` stacked a cell's blocks like any other paragraph until its UX pass
(`ui_web/src/text/table.rs`), and `grind-win32` did until the pass after that
(`ui_win32/src/text/geom.rs`'s `across` and `lay_out_table`, `ui_text_gtk`'s answers carried over
to GDI — and both desktop windows' copies are one `grind_text::flow` since the macOS shell's M1).
ᵠ Box-drawing rules, every column the same width. `grind_text::Faces` is handed a block's kind
and not its cell, so `grind-tui` builds a map of which block is in which cell once per frame and
reads it back — the shape `ui_text_gtk/src/view.rs`'s `Column` has, and required rather than
chosen: `Faces::of` is called while `App` holds its read lock. The map covers the view and a page
either side of it, which is the `ponytail` `doc/tui-shell.md` records.
ᶠ `:find`, then `n`/`N`, with every match marked in the line rather than only counted.
ʷ Edit ▸ Find… (Ctrl+F), Find Next/Previous (F3/Shift+F3) and Replace… (Ctrl+H) over the page, closed 2026-10-02: `grind_text::find` finds (ignoring case) and steps, the hit is selected, and `App::replace` writes (exactly, one undo step).
ʸ A second row in the find bar — *Replace with* and **Replace All** (`App::replace`: every occurrence, exact, one undo step); closed 2026-10-02. Its widget test skips without a display.
ᶻ Closed 2026-10-02, each a prompt over the core's own verb: the browser's palette verbs *Insert a table…*, *Bookmark this paragraph…* and *Name this paragraph's style…*, and the Windows Format menu's *Insert Table…* (Ctrl+Shift+T), *Bookmark Here…* (Ctrl+Shift+B) and *Paragraph Style Name…*. A table's size and place are `grind_text::table`'s — below the caret's block or table, never last — which the GNOME window asks too.
ᵍᵖ Move paragraph up/down and delete paragraph (`grind_text::blocks`, by caret and selection, not by address), 2026-10-02: the GNOME page's context menu, the browser's palette, the Windows Format menu. The GNOME context menu also has *Bookmark Here…* and *Paragraph Style Name…* (an alert with an entry). Insert-by-address is still the CLI's and the terminal's.
ᶜʰ Palette verbs *Insert a chart from the selection* (`verbs::insert_chart`, placed beside the table) and *Delete the last chart*; no edit, move or restyle — 2026-10-02. The terminal has `:chart` and `:chart!` (it writes the chart and draws none). Win32 has the same two as Data-menu items and **draws** charts (`sheet/chart.rs` over `grind_sheet::chart_paint`, the Mac's marks hoisted; a Wine render checks it).
ᵖⁱ *Insert a picture…* from the palette raises the file picker; the type is read from the bytes (`grind_text::picture::mime`) and `insert_below` places it — the same call GNOME, Windows and the Mac make. Closed 2026-10-02.
ᵗⁱ `:image <file>` puts the picture in below the caret's paragraph (`grind_text::picture`); the terminal still draws it as the placeholder character — the drawing row stays ○ (2026-10-02).
ˣ The palette is the find box, as over cells (Ctrl+F, or two characters typed after the verbs); picking a hit selects it, F3/Shift+F3 step, and Ctrl+H is *Replace in the document…* — closed 2026-10-02, over `grind_text::find`.
ᵗ The model reads a `table:number-columns-spanned`, writes it back with the covered positions it
implies, projects it as `span=` and (in `grind-text-gtk`) draws it merged. Nothing **creates**
one, and no client sets a column width, because the model carries no table style
(`doc/text-core.md`).

**Mac.** ¹ Through the system font panel (⌘T), whose answer becomes a family, a size and the
weight and slant, as one undo step (M7). ² Format ▸ Paragraph and the toolbar's
pop-up set all six levels, Title and Subtitle; ⌥⌘0–6 reach Body and the headings. ³ Edit ▸ Go To… (⌘L) asks for one; a sidebar
row or a Problems finding goes to one too, and Format ▸ Paragraph moves the selected paragraphs
up or down past their neighbour or deletes them — by where the caret is, not by address. ⁴ The sidebar's Outline section.

## 8. The divergences that matter, ranked

Everything here is reachable from the CLI, which is R9 doing its job. The order is by how much
of a client's own job is missing.

1. ~~**`grind-win32` can barely format a cell.**~~ **Closed** (2026-09-27, W12): the grid has
   the format strip decision 4 always gave it — bold, italic, alignment, both colours, the
   number presets, a decimal step and Clear, each a read of the active cell and one
   `App::set_style` or `App::set_format`, the same verbs in the Format menu. What is left is
   wrap and borders (§5 ˣ), which this window does not draw, and so does not offer.
2. ~~**`grind-sheet-gtk` has no borders control** (§5 ᵃ).~~ **Closed** (2026-10-02): a Borders
   toggle in the format bar, over `format::bordered`.
3. ~~**`grind-win32` cannot colour a run either**~~ — **stale, found 2026-10-02**: its format strip has had colour, highlight, family, size, strike, code and Clear since W5b/W10 and the pane draws the colours; this row and §7 were never updated. `grind-tui` still approximates one. Colour and
   highlight are the word processor's twin of row 1: the CLI writes them, loop C round-trips
   them, `grind-web` and `grind-text-gtk` draw them, and the Windows pane shows neither.
4. **Font family and size are a `grind-text-gtk`-and-CLI pair.** No other shell offers either
   control; the terminal has one font at one size by construction, and the browser and Windows
   panes both carry a family they cannot let anybody set.
5. ~~**No browser client has formula assist.**~~ **Two thirds closed**, after this file was first
   read: `ui_web/src/sheet/assist.rs` is autocomplete and signature hints over the same
   `grind_sheet::formula::assist` the GNOME and Windows shells use. **Point mode** — arrow keys
   building a reference into a half-typed formula — is still absent there and in `grind-win32`,
   and that file says why for the browser: it edits in exactly one place.
6. ~~**CSV is CLI-only.**~~ **Closed.** It was the widest *row* in this file — five ○ against one
   ● — and every client has both directions now. What is left of it is a *narrower* row, and one
   shape rather than five: no window offers the import's seven options (§6 ʲ). Each imports with
   `csv::Import::sniffed` and exports at a name whose extension picks the delimiter, which put
   the two answers a window cannot skip — which delimiter, which range — in the core where four
   shells share them rather than in four shells that could differ. The decode rule moved with
   them: `csv::decode` and `csv::NOT_UTF8` are why "this file is not UTF-8, run `iconv`" is one
   sentence everywhere instead of five.
7. **Column widths, row heights, hidden tracks, filters and defined names are CLI-and-GNOME
   only.** Every other client honours all five faithfully and can create none of them.
8. **Charts are CLI-and-GNOME to author, and only the browser joins them in drawing one.**
9. **An image can be inserted from the CLI and from `grind-text-gtk`**, and only those two
   clients draw one.
10. ~~**Two clients out of five draw a table.**~~ **Closed** (2026-09-27): all five draw the grid
    now — `grind-text-gtk` and the Windows pane in pixels, the browser as a CSS grid and
    `grind-tui` in box-drawing characters, each over the same `Faces` seam — and every one of
    them edits inside a cell, because a cell holds blocks (§7 ᵖ). What is left is narrower:
    the browser and the Windows pane cannot *insert* one (§7), and no client can merge cells or
    set a column width (ᵗ).
11. ~~**Find and replace over cells were the core's gap.**~~ **Closed** (2026-09-28): `App::find`
    and `App::replace` exist for the spreadsheet (`grind_sheet::find`), and every client reaches
    them — `grind sheet find`/`replace`, `grind-tui`'s `:find` and `:s/old/new/`, the GNOME
    window's find bar, the browser's palette and F3, and the Windows grid's Edit menu and F3 —
    with `grind_sheet::find::step` deciding where the next hit is for the three that step. What
    is left is narrower, and over *text*: find and replace there exist on the CLI and in the
    terminal, and find alone in `grind-text-gtk`.
12. **Text undo is not on the CLI** (§2 ᶜ) — the one row where a shell is ahead of the CLI, and
    it is a decision about `grind_text::Action` rather than about the CLI.
13. **`grind-sheet-gtk` has no cross-app handoff** (§2 ᶠ), where its twin does.

**Amended on 2026-10-02 for the Mac**, which is now on the authoring side of five of these rows —
written and type-checked, not yet run (§1): it sets and draws borders (row 2), has point mode
(row 5), sizes tracks by dragging and fitting, turns a filter on and chooses its values, and
defines, redefines, renames, inlines and deletes names — the renaming and inlining first in any
window (row 7) — inserts and draws charts (row 8), and inserts and draws pictures (row 9).

## 9. Absent from every client

Not a parity problem — a feature line. Each has its row in `doc/not-doing.md` or a gate in a
shell document, and none of them is reachable from the CLI either.

**Spreadsheet.** Conditional formatting · merged-cell rendering (the model carries no spans) ·
freeze panes · sort · printing · pivot tables · macros
(`doc/not-doing.md` §1 — the generator is the answer, and `grind build` is a CLI verb by R11).
**Autosave is no longer in this list**: the macOS shell has it, by platform convention, and no
other client does (`doc/not-doing.md` §3, *Autosave*).

**Word processor.** Footnotes · fields (`text:page-number`, `text:date`, …) · style
*definitions* (a named character style is kept and never interpreted) · pages · printing · an
image anchored mid-sentence, which draws as the placeholder character everywhere · a table's own
style (column widths, borders), merging cells from any client, and `table:formula` in a cell —
the last of which is gated on `doc/odt-format.md` §5's unanswered question about whether a
Writer table's formula is OpenFormula at all.

**Both.** Pagination and RTL, both gated — RTL by explicit decision in `doc/text-layout.md`.
Editing the code view (`doc/dsl.md` §6.4). `grind test` is built (D8) and, like `grind build`,
is the CLI's alone by R11: no shell links the generator.

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
sed -n '/pub static MENUS/,/^];/p' ui_mac/src/menu.rs      # and Command::applies, and tools.rs
grind sheet --help; grind text --help

# 3. The cross-check, which is what actually caught §8's first two rows: a verb table can
#    promise anything, and a shell that never calls the core method cannot deliver it.
for d in ui_sheet_gtk ui_text_gtk ui_tui ui_web ui_win32 ui_mac; do
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

**Amended on 2026-10-02/03** after a gap-closing pass over all clients. Closed: find and replace over
the text pane in Win32, the browser and (replace) GNOME; bookmark / named style / insert table in the
browser and Win32; borders drawn and settable in Win32 and settable in GNOME; fill, hide, sizes,
define name, evaluate, copy value, formula-to-value in Win32; the same family in the browser and
terminal; rename/inline name in GNOME and the terminal. The Windows text column and the browser's
hide/filter cells were **already stale** and were corrected. Shared pieces hoisted into the cores so
shells cannot diverge: `grind_text::{find, table}`, `grind_sheet::{verbs, nav::fills,
Filter::over_selection, look::border_strokes, format::bordered}`. Still open and heavy: charts
(authoring in Win32/TUI/Web, drawing in Win32), zoom and drag-resize/autofit outside GNOME and Mac,
point mode outside GNOME and Mac, wrapped-text drawing in Win32/TUI, the text-pane image story in
the terminal, and the text pane's insert/delete/move-block verbs in the windows.

**Amended on 2026-10-01** for the Mac column (`doc/macos-shell.md`'s M11), read out of `ui_mac`
through M10 by the same three steps — every row judged against the code, its notes numbered so
they cannot collide with the lettered ones — and none of it yet seen running. Nothing else was
re-derived.

**Amended on 2026-09-22** for §5's three locale rows — a document now states its own locale, the
GNOME window and the CLI set it, and every client honours it since the core does the spelling —
and for §6's chart rows: a chart read from a table, its own title and legend (which the browser
draws too), and the GNOME window's preview. Nothing else was re-derived.

**Amended on 2026-09-12** for the CSV rows (§6, §8 row 6), which every GUI shell now has, and for
§8 row 5, which the browser shell's formula assist narrowed to point mode after the first
reading. Amended again the same day for §2's three start rows: the browser and Windows shells
open on a **welcome screen** rather than on an empty spreadsheet, which turned "New, empty
document" from a gap into a capability in the web column and added a way to start the *other*
kind in place. Nothing else was re-derived that day, so everything below still carries the date
it was read on.

**Read on 2026-09-10**, at `main`. The clients as of then: `grind sheet` complete through phase
9, `grind text` through S10, the GNOME spreadsheet through M10 plus charts, filters and the
chrome rework, the Windows shell through W10, `grind-text-gtk` through the showcase pass that
gave it a formatting bar, a clipboard, block structure and Insert Picture (`doc/text-shell.md`),
and `grind-tui` through the pass that gave it chrome, column widths, a formula-completion band,
cell search, the track and name verbs, CSV both ways, bookmarks, `:move`, an outline pane and a
table drawn as a grid (`doc/tui-shell.md`).
