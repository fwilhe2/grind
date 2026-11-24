<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# The feature matrix — every capability, in every client

> **Whatever any GUI can do, the CLI can do. A UI-only feature is a bug.**
> — `doc/plan.md` rule 4
>
> **R10 allows per-shell feature gaps and requires them to be named.**
> — `doc/suite.md`

Rule 4 is checked mechanically, and only down one column: `cli/tests/parity.rs` reads
each `App`'s methods (`sheet/src/`, `text/src/`) against `doc/cli-parity-sheet.md` and
`doc/cli-parity-text.md`, so **the CLI's column cannot silently lose a row**. Nothing checks the
other six. Each shell names its own gaps in its own document, which is the right place for the
*reason* — and it is the wrong place to see that one window lacks what another has, because
no reader holds six gap lists in their head at once.

This file is that cross-cut: one row per capability, one column per client, read out of the
code rather than out of the six shell documents. It is **descriptive, not normative** — where
it disagrees with `doc/sheet-shell.md`, `doc/text-shell.md`, `doc/tui-shell.md`,
`doc/web-shell.md`, `doc/windows-shell.md` or `doc/macos-shell.md`, those are right and this is
stale. §10 says how to re-derive it, and why that step is worth keeping.

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

The Mac column's notes are numbered, under each table, so they cannot collide with the lettered
ones.

## 2. Suite-wide

| | CLI | Sheet GTK | Text GTK | TUI | Web | Win32 | Mac |
|---|---|---|---|---|---|---|---|
| Opens a spreadsheet | ● | ● | — | ● | ● | ● | ● |
| Opens a text document | ● | — | ● | ● | ● | ● | ● |
| Reads all three forms (`.ods`/`.fods`, `.odt`/`.fodt`, `.grind`) | ● | ● | ● | ● | ● | ● | ● |
| Writes all three forms | ● | ● | ● | ● | ● | ● | ● |
| Flat-first default (`doc/flat-first.md`) | ● | ● | ● | ● | ● | ● | ● ¹ |
| New, empty document | ● | ● | ● | ● ᵗⁿ | ● | ● | ● |
| A **welcome screen** with no document open | — | ○ | ○ | ● ᵗʷ | ● | ● | ● |
| New document of the *other* kind, in place | — ⁱ | ○ | ○ | ● ᵗⁿ | ● | ● | ● ² |
| Undo / redo — spreadsheet | ◐ ᵇ | ● | — | ● | ● | ● | ● |
| Undo / redo — text | ○ ᶜ | — | ● | ● | ● | ● | ● |
| Code view — the projection, read-only | ● | ● | ● | ● | ● | ● | ● |
| Check Document — `lint` findings, each a jump | ● | ● | ● | ● | ● | ● | ● ³ |
| Go to an address | ● | ● | ● | ● | ● | ● | ● ⁴ |
| Key list / help | ● | ● | ● | ● | ● ᵈ | ● | ● ⁵ |
| About / build stamp | ● | ● | ● | ● ʰ | ● ʷᵃ | ● | ● ⁶ |
| Recent files | — | ● | ● | ● ᵗʳ | ● ʷᶜ | ● ʷʳ | ● |
| A library on `grind serve` — open, save back, history (`doc/server.md`) | ● ˢᵛ | — ˢᵛ | — ˢᵛ | — ˢᵛ | ● ʷˢ | — ˢᵛ | — ˢᵛ |
| Opening the *other* document kind | ● ᵉ | ● ᶠ | ● ᵉ | ● | ● | ● | ● |
| Assertable headless output | stdout | `--render-to` PNG | `--render-to` PNG | `TestBackend` | `smoke.js` (jsdom) | `--render-to` BMP | `--render-to` PNG, `--drive` |
| `.deb` + `.rpm` | ● | ● | ● | ● | — | — | — ⁷ |
| Accessibility floor | — ᵍ | announce | announce | terminal | ARIA labels | system caret | text area, announce |

ᵗⁿ `:new [sheet|text]` and `:open <file>` (`!` to discard unsaved work) replace the pane in the same terminal; the event loop swaps it (`app::Switch`) — checked in a pty.
ᵃ `grind-tui --sheet` / `--text` starts an empty document; there is no in-session "new".
ⁱ `grind sheet new` / `grind text new` make one, which is the same capability; "in place" is a
question only a window with one document open at a time has. The two GTK shells are one document
type each (`doc/suite.md`), so the row means nothing there; `grind-tui` has no "new" at all — see
ᵃ — which makes it the one client where the welcome screens' verbs have no equivalent.
ᵇ Only with `--session`, which carries `grind_sheet::Action` between invocations.
ᶜ **Named**: `grind_text::Action` is not serialisable, so there is no text session
(`doc/cli-parity-text.md`, "History"). Every shell can undo a text edit in process.
ᵈ The palette shows each verb's key beside it, and *Keyboard shortcuts* (F1) lists the showing pane's moves and every row's key in a dialog.
ᵉ The four clients marked plainly ● simply open it — one binary, both kinds. The CLI routes on
`grind_core::kind` at the suite level (`info`, `lint`, `convert`); `grind-text-gtk` cannot open
one and says so, raising a banner — *"This is a spreadsheet"* + **Open in Sheet**, which launches
the other binary.
ᶠ The ODS reader is tolerant by construction and would read a `.fodt` as a document with no
sheets, so `grind-sheet-gtk` asks `grind_core::kind` on the worker thread before reading,
and a text document — or a `.docx` or `.md`, which say nothing in their bytes — raises its twin's
banner the other way round: *"report.fodt is a text document."* + **Open in Text**, which
launches `grind-text-gtk` (`is_text`, unit-tested; the banner itself not yet seen on a display).
ʰ `:about`, on the status line — there is no dialog to put it in.
ᵍ A pipe is the accessible surface, which is `doc/view-modes.md` §4.6's argument for why the
CLI matters most exactly where a GUI's whole output is colour.

ᵗʳ The welcome pane lists the nine most recent, `1` to `9`, from `$XDG_STATE_HOME/grind/recent` — every file opened or written (checked in a pty).
ʷᶜ The browser keeps a *copy* of the four last opened or saved, each up to 768 KiB, in `localStorage` — a page has no path to reopen — listed on the welcome pane and in the palette, with *Forget the recent documents* (smoke-checked).
ʷʳ Every document opened or saved is handed to `SHAddToRecentDocs`, so it is in the taskbar jump list and every file dialog's Recent — the platform's own list, with none of ours (build-checked only).
ˢᵛ The library is a directory of files, so every client that opens a file opens a document in it, and `git log` is its history; only the browser needs the server to reach them.
ʷˢ Only when `grind serve` served the page: the welcome pane lists its documents, Save writes back (412 → *replace theirs?*), and the palette adds *Save to the server as…*, rename, delete, *Download a copy* and, in a git work tree, *Earlier versions…* with restore (`smoke-server.js`).
ʷᵃ *About this build* in the palette says `grind_core::build_info::describe` on the message line — no dialog to put it in.

**Mac.** ¹ An untitled document saves flat: the document answers `fods` or `fodt` itself,
since the type it is declared under would have said `.ods`. ² A window of its own beside this
one — one window per document is decision 5. ³ The sidebar's Problems section rather than a
dialog, every row a jump. ⁴ The grid's name box (⌘L) takes any address; on a page ⌘L
asks for one in an alert, and its sidebar's outline and bookmarks are jumps too. ⁵ Every menu item shows
its key, and Help ▸ Search finds any item — the platform's own key list. ⁶ The standard About
panel, told `grind_core::build_info`'s stamp: the version, the commit and the build, and when. ⁷ A universal,
ad-hoc-signed `Grind.app` in a DMG instead.

## 3. Spreadsheet — navigation and selection

| | CLI | Sheet GTK | TUI | Web | Win32 | Mac |
|---|---|---|---|---|---|---|
| Move by cell, row, page, document | — | ● | ● | ● | ● | ● |
| Jump to the edge of a block (Ctrl+arrow) | — | ● | ● ᵉᵈ | ● ᵉᵈ | ● | ● |
| Jump to the start or end of the sheet | — | ● | ● | ● | ● | ● |
| Select a rectangle | ● ᵃ | ● | ● | ● | ● | ● |
| Select whole rows / columns from a header | ● ᵃ | ● | ● ᵉᵈ | ● ʷʰ | ● | ● |
| Select the whole sheet | ● ᵃ | ● | ● ᵉᵈ | ● | ● | ● |
| Name box / address field | ● | ● | ● | ● | ● | ● |
| Go to a defined name | ● | ● | ● | ● | ● | ● |
| Skip a hidden or filtered row while moving | — | ● ʷᵉ | ● ᶜ | ● ʷᵉ | ● | ● |
| Sheet switching | address | tab strip | `:sheet` | tab strip | tab strip + Ctrl+PgUp/PgDn | sidebar |
| Zoom | — | ● | ○ | ● ʷᶻ | ● ʷᵐ | ● |

ᵉᵈ Over `grind_sheet::nav`'s edge rule: the browser's Ctrl+arrow (Shift extends); the terminal's `w b } {` and Ctrl+arrows, with `V` (row), Ctrl+V (column) and Ctrl+A (sheet) as visual selections of the used part — no header to click in a terminal.
ʷʰ A click on a column's letter or a row's number selects the whole track (the smoke test clicks one); dragging a column header's right edge sizes the column through its `<col>` and writes one `set_col_width` on release — the drag is build-checked, jsdom having no layout. Rows are sized by the *Row height…* verb.
ᵃ A range is an argument, not a gesture: `A1:C9`, `A:A`, a sheet-qualified form.
ʷᵉ The GNOME grid applies `nav::onto_visible` after its move (closing the gap `doc/sheet-shell.md` named; lint-checked only). The browser's motions are `grind_sheet::nav::moved` now, then `nav::onto_visible` over the hidden and filtered tracks — the rule the Windows and Mac grids use. It replaces the browser's own `keymap::moved`, so selection and Shift-extend behave as they do there.
ʷᶻ Ctrl+`+`/`-`/`0`, Ctrl+wheel and palette verbs scale the table and the charts over it with CSS `zoom` (25%–400%); a reading, never stored. Hit-testing is the DOM's; how many cells fit divides the room by the factor. Compile- and smoke-checked, not seen.
ʷᵐ View ▸ Zoom In/Out/Reset (Ctrl+`=`, Ctrl+`-`, Ctrl+wheel), 25%–400%: every track is scaled by the zoom (`Sizes::scaled`), and the cell font, wrapped lines, row heights, borders and charts follow it. A Wine render at 150% checked the cells, text and a chart.
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
| In-cell editor | — | ● | ○ ᵃ | ◐ ᵃ | ● | ● |
| Formula bar | — | ● | ◐ ᵃ | ● | ● | ● ¹ |
| Clear a cell or a range | ● | ● | ● | ● | ● | ● |
| Clear only the formula, keeping the value | ● | ● ᵍᶠ | ● ᵗᵘ | ● | ● | ● |
| Paste a rectangle of tab-separated rows | ● | ● | ● | ● | ● | ● |
| System clipboard (cut / copy / paste) | — | ● | ◐ ᵇ | ● | ● | ● |
| Copy Value — the formatted result, not the formula | ● | ● | ● ᵗᵘ | ● | ● | ● |
| Fill down / fill right | ● | ● | ● | ● | ● | ● |
| Fill one cell across a rectangle (references shifted) | ● | ● | ● | ● | ● | ● |
| Recalculate | ● | ● | ● | ● | ● | ● ² |
| Stale-value warning | ● | ● | ● | ● | ● | ● |
| Evaluate a formula without storing it | ● | ● ᵈ | ● | ● ᵖᵃ | ● | ● |
| Selection arithmetic — Sum, Count, Average | ● ᵉ | ● | ● ᵉ | ● | ● | ● |
| Autocomplete while typing a formula | ● ᶜ | ● | ● | ● | ● | ● |
| Signature hint for the call the caret is in | ● ᶜ | ● | ● | ● | ● | ● |
| Point mode — arrow keys build a reference | — | ● | ● ᵖᵐ | ● ᵖᵐ | ● ᵖᵐ | ● |
| Friendly formulas — `Sum(Number: B2:B7)` | ● | ● | ● ᵉˣ | ● ᵉˣ | ● | ● |
| Explain a nested formula, unfolded | ● | ● | ● ᵉˣ | ● ᵉˣ | ● | ● |
| Every function as a browsable list | ● | ● ᶠᵍ | ● ᶠᵍ | ● ᶠⁿ | ● | ● |
| Read a formula through the document's names | ● | ● | ● | ● | ● | ● |
| Every calculated cell, searchable | ● | ● | ● ᶜᵃ | ● ᶜᵃ | ● ᶜᵃ | ● |
| Find over cells | ● | ● ᵍ | ● ᶠ | ● ʰ | ● ⁱ | ● |
| Replace over cells | ● | ● ᵍ | ● ᶠ | ● ʰ | ● ⁱ | ● |
| Misspelt words in text cells listed, with addresses (`doc/spelling.md`) | ● | ● | ● | ● ʷˢ | ● | ● |
| Misspelt words in text cells underlined, suggestions, Add to Dictionary | ◐ ˢᶜ | ● | ● ˢᶜ | ● ʷˢ | ● | ● ˢᶜ |
| Spelling off and on, remembered for every spreadsheet | ● ˢᶜ | ● | ● | ● ʷˢ | ● | ● ˢᶜ |

ʷˢ **The browser's spelling** (`ui_web/src/spelling.rs`, `doc/spelling.md`). The same checker and rules, with `grind-spell` linked *without* its bundled word lists (+0.4 MB of module): the page fetches `dictionaries/<tag>.aff` and `.dic` for the one language wanted, the first time a document opens with spelling on. A wavy red line in both panes (the sheet's under a cell's text); the palette lists a correction, *Ignore* and *Add to your dictionary* for the word at the caret, or in the active cell, *Go to the next misspelling* in either pane, and the *Spelling:* rows — Automatic, each language, Off — remembered per kind of document in `localStorage`, as are added words. `smoke.js` drives it against the real dictionaries.
ᵖᵐ Where `ref_eligible` says a reference could go, the four arrows point (one cell, no range extension) and write the address into the edit line; the pointed cell is drawn in magenta; the browser does the same in the formula bar (UTF-16 offsets through `grind_core::utf16`), writing the address without highlighting the cell — compile- and lint-checked only. Win32 does it in the in-cell `EDIT` through `EM_REPLACESEL` (Enter mode starts one; Edit mode keeps the caret) — also checked only by build.
ᶠⁿ Type two letters of a function's name or plain-English name in the palette: up to five rows follow the verbs, and picking one starts an edit seeded `=NAME(`. A search, not a scrolling list.
ᶠᵍ *Functions…* in the GNOME palette (a searchable dialog) and `:functions [text]` in the terminal (a list pane): name, plain-English name, brief and category from `funcs::catalog`; picking one starts an edit seeded `=NAME(`.
ᶜᵃ `:calc [text]` (a list pane) and Data ▸ Find a Calculation… (a prompt, then a list) over `App::calculations`; each row a jump. The browser's Ctrl+K ▸ *Find a calculation…* asks a word and lists the matches in the problems pane's shape (`problems::Listing`), the function tally in its header, each row a jump; smoke-checked.
ᶠᵒ `:formulas` in the terminal, a palette verb in the browser, View ▸ Show Formulas in Windows and GNOME (the GNOME one draws the cell text only, and is lint-checked rather than seen): each formula cell shows its formula text (display syntax) instead of its result — a reading, nothing is written.
ˢᶜ (`doc/spelling.md`, "In a spreadsheet"). Only typed text is checked — never a number, a date or a formula's result — and the ranges come in the `Viewport`, so every client marks the same words. The CLI has nothing to underline: `grind sheet spell [range] --suggest`, `sheet correct`, and `sheet spelling auto|off|<lang>`, the remembered switch `sheet lint` and `grind lint` follow. The terminal underlines in red only the letters a column shows, `]`/`[`, `:fix`, `:spell`. The GNOME window's wavy line, cell menu, View ▸ Check Spelling (Shift+F7) and Spelling Language; Windows' the same, checked under Wine; the Mac's dotted red line, Edit ▸ Spelling ▸ Check Spelling While Typing and the context-menu rows, type-checked and not yet run. 
ˡᵒ `:locale [tag|none]`, a palette verb and Data ▸ Document Locale…: a tag such as `de-DE` through `verbs::locale`, then `App::set_locale` — one undo step.
ʲʷ In words rather than checkboxes, through `csv::Import::amended` (`delimiter=semicolon locale=de-DE text formulas trim no-dates`): `:csv-in <file> with …` in the terminal, *Import CSV with Options…* in GNOME's palette (an alert, then the picker), *Import CSV with Options…* in the Mac's File menu (type-checked only), *Import CSV with options…* in the browser's palette and File ▸ Import CSV with Options… in Windows — the options asked first (the browser) or after the file (Windows).
ᵍᶠ *Replace Formulas with Values* in GNOME's palette (`verbs::formulas_to_values` over the selection; lint-checked only).
ʷⁿ A *Names* menu: Define for the Selection, Rename, Inline into Formulas, Delete — typed by name in a prompt, over the core's own calls.
ᵃ The TUI edits on a formula line rather than in the cell. The browser's text lives in the formula
bar, and the cell being edited draws it with the bar's caret where it is, so typing
reads as in the cell; a click in that cell does not move the caret. Both are `App::enter`
underneath, so the *rule* is identical and only the surface differs.
ᵇ A vi register, also handed to the system clipboard by OSC 52 where the terminal honours it, and
a paste from the terminal put at the active cell, or typed into the line being edited.
`p` still puts the register: reading the clipboard back is a query many terminals
refuse (`doc/tui-shell.md`).
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

| | CLI | Sheet GTK | TUI | Web | Win32 | Mac |
|---|---|---|---|---|---|---|
| Bold, italic | ● | ● | ● | ● | ● | ● |
| Underline, strikethrough (cells) | ● ᵘˡ | ● ᵘˡ | ● ᵘˡ | ● ᵘˡ | ● ᵘˡ | ● ᵘˡ |
| Indent (cells) | ● ⁱʳ | ● ⁱⁿ | ● ⁱⁿ | ● ⁱⁿ | ● ⁱⁿ | ● ⁱⁿ |
| Rotation (cells) | ● ⁱʳ | ● ʳᵗ | ● ʳᵗ | ● ʳᵗ | ● ʳᵗ | ● ʳᵗ |
| Alignment | ● | ● | ● | ● | ● | ● |
| Wrap text | ● | ● | ● | ● | ● ʷʷ | ● |
| Borders | ● | ● ᵃ | ● | ● | ● ʷᵇ | ● |
| Text colour, cell background | ● | ● | ● | ● | ● | ● |
| Clear formatting | ● | ● | ● | ● | ● | ● |
| Number formats — the eight presets | ● | ● | ● | ● | ● | ● |
| Decimal places, grouping, currency symbol | ● | ● | ◐ ᵇ | ◐ ᶜ | ◐ ʷ | ◐ ² |
| Read a cell's style / format back | ● | ● | ● | ● | ● | ● |
| A live sample of a number format before it is set | — | ● ᶠ | ● ᵗˢ | ● ʷˢ | ● | ● |
| Set the document's own locale (`doc/ods-format.md` §5.2) | ● | ● ᵍ | ● ˡᵒ | ● ˡᵒ | ● ˡᵒ | ● |
| **Honours** the document's locale — shown and typed | ● | ● | ● | ● | ● | ● |
| **Drawn**: bold, italic, alignment | — | ● | ● | ● | ● | ● |
| **Drawn**: underline, strikethrough | — | ● | ● | ● | ● | ● ᵘˡ |
| **Drawn**: indent | — | ● | ● ⁱʳ | ● | ● | ● |
| **Drawn**: rotated text | — | ● ⁱʳ | ○ | ● ʳᵗ | ● ʳᵗ | ● ʳᵗ |
| **Drawn**: colours | — | ● | ◐ ᵈ | ● | ● | ● |
| **Drawn**: borders | — | ● ᵉ | ◐ ᵇᵈ | ● | ● ʷᵇ | ● ³ |
| **Drawn**: wrapped text | — | ● | ● ˡⁿ | ● | ● ʷʷ | ● ¹ |
| **Drawn**: a number too wide for its column is `###`, never cut (`numfmt::overflow`) | — | ● | ● | ● | ● | ● |
| **Drawn**: text too wide runs into empty neighbours, a number never does | — | ● | ◐ ˢᵖ | ● | ● | ● |
| **Drawn**: a cell's own font size | — | ● | ○ | ● | ● | ● |
| **Drawn**: tabular figures, so a column of numbers lines up | — | ● | ● | ● | ● ᵗⁿ | ● |
| **Drawn**: a plain number (the General format) narrows to its column first — fewer decimals, then scientific (`Viewport::narrower`) | — | ● | ● | ● | ● | ● |

ˢᵖ Left-aligned text only, rightwards: a row is drawn cell by cell from the left, so centred or right-aligned text that would run leftwards is cut with `…`.
ᵗⁿ Segoe UI's digits are one width already, and GDI has no OpenType features to ask with; a Wine render checked it.
ᵃ A *Borders* toggle in the format bar writes `format::bordered` — a hairline on every edge, or none — over the selection, the same call the terminal, the browser and the Mac make; a per-edge width or colour is still `grind sheet style --border`'s.
ᵇ `:format number [n]` takes a decimal count, `:format currency [eur|usd|gbp]` one of the three
currencies `numfmt::CURRENCIES` offers and `:group` the thousands separator; any other symbol and the locale are `grind sheet format`'s.
ᶜ More / fewer decimals, the three currencies of `numfmt::CURRENCIES` as three menu entries, and *Group thousands*. No other symbol.
ʷ One decimal more or fewer at a time (the strip's `-.0`/`+.0`, and Format ▸ Increase/Decrease
Decimals), and the three currencies of `numfmt::CURRENCIES` — Format ▸ Currency and the cells'
context menu, a currency cell keeping its own decimals and grouping (`grind_sheet::format::currency`,
hoisted from `ui_win32/src/sheet/currency.rs`), and Format ▸ Use 1000 Separator, checked by the cell
(`format::grouped`). No other symbol.
ˡⁿ A wrapped text cell is broken at its column's width by `grind_core::layout::wrap` in terminal cells, and its row is as many lines as its tallest cell, up to eight; the rows drawn are those whose lines fit (`TestBackend`-checked).
ʷᵗ The rows a wrapped or larger cell grows are measured by `grind_sheet::autoheight` on the stylesheet's own line height (`layout::Drawn`), once per change, and given that height — so the rows counted onto the surface are the rows drawn there (smoke-checked).
ʷʷ Format ▸ Wrap Text; the grid breaks a wrapped cell with `grind_core::layout::wrap` over GDI measurements (`sheet/measure.rs`) and grows rows through `grind_sheet::autoheight` (hoisted from the Mac) — checked by a Wine render.
ʷᵇ Format ▸ All Borders / Remove Borders (`format::bordered`), and the grid draws every edge through `look::border_strokes` — the Mac's geometry, hoisted — checked by a Wine render. `dashed` and `dotted` are cut into runs.
ˣ Named in `doc/windows-shell.md`'s "What it will not do": this window draws neither wrapped text
nor borders, and a control whose effect cannot be seen is not offered.
ᵈ Sixteen terminal colours, nearest match — the medium, not a gap (`doc/tui-shell.md`).
ᵇᵈ A left or right border as `│` in the column separator, in its colour; a top or bottom one is not drawn, having no line between rows to go in (`doc/tui-shell.md`).
ᵉ Through `look::border_strokes`, as Windows and the Mac draw them: `double` as two lines, `dashed` and `dotted` as runs in step across cells, LibreOffice's `dash-dot` kinds as dashes.
ᶠ Above the number popover's settings: the active cell as they would show it, from
`App::shown_as`, the renderer's own answer — and the strip's button face says what the cell is
formatted as (`123`, `%`, `€`, `Date`).
ᵍ *Document Settings…*, applied at once as one undo step. **Honouring** a locale costs a client
nothing, which is why that row is full: every cell's text comes out of `App::get_viewport`
already spelled the document's way, and every typed number goes through the core's typing
rule. A new document in the GNOME window states the desktop's locale; `grind sheet new` states
none unless told.

ᵘˡ A cell's `style:text-underline-style` and `style:text-line-through-style`: read, written, projected (`underline=`/`strike=`) and carried from `.xlsx`, and set by `grind sheet style --underline --strike`. Every window draws them — Pango attributes, terminal modifiers, CSS `text-decoration`, `LOGFONTW`'s own two switches, and on the Mac two fills placed from the portable metrics. Every window sets them too, as `grind_sheet::format::Toggle::Underline`/`Strike` — any style but `none` reads as on, a press writes `solid` or takes it off: two toggles beside Bold and Italic on the GNOME format bar, the browser's tool row (Ctrl+U) and the Windows strip (Ctrl+U, Ctrl+Shift+X, and the Format menu), `_`/`~` in Visual mode and `:underline`/`:strike` in the terminal, and the Mac's toolbar and Format ▸ Font menu, ungreyed on the grid. The Windows grid draws italic now too, which it did not before.

ⁱʳ `fo:margin-left` and `style:rotation-angle`: read, written, projected (`indent=`/`rotate=`), carried from `.xlsx` (`doc/xlsx-format.md` §4.14) and set by `grind sheet style --indent --rotate`. The terminal indents in whole columns. Rotated text is drawn turned about the cell's middle in every window but the terminal, which draws it level and keeps the angle.
ʳᵗ `grind_sheet::format::ROTATIONS` — Excel's four turns (45° either way, up, down) and level again, through `format::rotated`: a *Text Rotation* menu on the GNOME format bar (lint-checked only), `:rotate 45|up|down|none` in the terminal, which keeps the angle and draws the text level; five palette rows in the browser, which turns the text with a CSS transform inside the cell (smoke-checked); Format ▸ Text Rotation… on Windows, a chooser, and the grid draws it in a turned `LOGFONTW` clipped to the cell (seen in a Wine render); Format ▸ Text ▸ Rotation on the Mac, its five items ticked by the cell, drawn as an `Op::Turned` about the cell's middle (type-checked and unit-tested, not seen).
ⁱⁿ Increase and Decrease Indent, whole levels of `grind_sheet::format::indented` — 0.2646cm a level, the step the oracle writes for an imported `indent="1"` (`doc/xlsx-format.md` §4.14), so a level set here is a level an imported workbook already has: two buttons beside the alignments on the GNOME format bar and the browser's tool row (and two palette rows there), two icon buttons on the Windows strip and Format ▸ Decrease/Increase Indent, `>`/`<` in Visual mode and `:indent`/`:outdent` in the terminal, and ⌘[/⌘] in the Mac's Format ▸ Text with a toolbar group.

**One rule for a number that does not fit.** Part of a number is a different number —
`2026-08-16` cut to `2026-08-1` is the first of August, `3,710.00 €` cut to `3,710.0…` has lost a
digit and its currency — so a number, date or time too wide for its column is drawn as `###`, as
many whole hashes as the column holds, in every client: `grind_sheet::numfmt::overflow`, measured
in each shell's own unit and decided by the **value** being a number, never by where it is
aligned. Text that does not fit ends in `…`. **A plain number narrows before it hashes**:
one in the General format — no format of its own — is first drawn with fewer
decimals and then in scientific notation, the first of `Viewport::narrower`'s spellings that
fits, which is what LibreOffice shows (`154.719066107646` in a narrow column is `154.719`, not
`###`; `doc/ods-format.md` §5.2). A number whose format states its decimals still hashes.

**Mac.** ¹ Broken at the column's width and drawn as lines, and a row with no height of its own
grown to hold them, as the GNOME window grows one. ² The toolbar's two steps and Format ▸ Number's, and Format ▸ Number ▸ Group Thousands and the three currencies of `numfmt::CURRENCIES`, each ticked by the cell (`format::grouped`, `format::currency`). No other symbol. ³ Each edge its own width and colour, `double` as two lines, `dashed` and `dotted` as runs of fills. Format ▸ Borders sets a hairline round every selected cell or takes them all away.

## 6. Spreadsheet — structure, charts and interchange

| | CLI | Sheet GTK | TUI | Web | Win32 | Mac |
|---|---|---|---|---|---|---|
| Add / rename / delete a sheet | ● | ● | ● | ● | ● | ● |
| Rename carries every reference with it | ● | ● | ● | ● | ● | ● |
| Set a column width or row height | ● | ● | ● | ● ᵖᵃ | ● | ● |
| Drag a track edge to resize | — | ● | ○ | ● ʷʰ | ● ʷᵈ | ● |
| Autofit a column | ● ᶜᶦ | ● ᵍᶠ | ● ᶠⁱ | ● ʷᶠ | ● ᶠᶜ | ● ᵐᶠ |
| Fit Content to Cells — every column and row, one undo step | ● ᶜᶦ | ● ᵍᶠ | ● ᶠⁱ | ● ʷᶠ | ● ᶠᶜ | ● ᵐᶠ |
| Row auto-height from content | — | ● | ● ˡⁿ | ● ʷᵗ | ● ʷʷ | ● |
| **Honours** the document's widths and heights | ● | ● | ● ᵃ | ● | ● | ● |
| Hide / unhide a row or column | ● | ● | ● | ● | ● | ● |
| **Honours** hidden tracks | ● | ● | ● | ● | ● | ● |
| Create or clear a filter | ● | ● | ● ᵗᵘ | ● | ● | ● |
| **Honours** a filter | ● | ● | ● | ● | ● | ● |
| Merge / unmerge cells | ● | ● ᵐᵍ | ● ᵐᵍ | ● ᵐᵍ | ● ᵐᵍ | ● ᵐᵍ |
| **Draws** a merge as one cell | ● ᵐᵈ | ● | ● | ● | ● | ● |
| Tick a checkbox (click, Space) | ● ᶜᵇ | ● | ● | ● | ● | ● |
| Add or remove a checkbox | ● ᶜᵇ | ○ | ○ | ○ | ○ | ○ |
| **Draws** a conditional-format rule | ● ᶜᶠ | ● | ● | ● | ● | ● |
| Add or remove a conditional-format rule | ● ᶜᶠ | ● ᶜᶠ | ● ᶜᶠ | ● ᶜᶠ | ● ᶜᶠ | ● ᶜᶠ |
| Define, redefine or delete a name | ● | ● | ● | ● ᵖᵃ | ● ʷⁿ | ● ³ |
| Rename a name, carrying every use (§6.5) | ● | ● ᵖᵇ | ● ᵗᵘ | ● ᵖᵃ | ● ʷⁿ | ● ³ |
| Inline a name into every use (§6.5) | ● | ● ᵖᵇ | ● ᵗᵘ | ● ᵖᵃ | ● ʷⁿ | ● ³ |
| Extract a number into a named input cell (`doc/calc.md`) | ● | ● | ○ | ○ | ○ | ○ |
| The tape, a scratch calculator that saves as a sheet (`doc/calc.md`, experimental) | ● | ● | ● | ● | ● | ○ |
| Sees the document's defined names | ● | ● ᶜ | ● ᵈ | ● ᵉ | ● ᶠ | ● ¹ |
| Add, edit, remove, move or restyle a chart | ● | ● | ● ᶜʰ | ● ᶜʰ | ● ᶜʰ | ● ⁵ |
| **Take hold of** a chart — select it (accent outline, eight handles), drag to move, a handle to resize, Delete, a menu on it (`doc/chart-handling.md`) | ● ᵍʳ | ● | ◐ ᵍʳ | ● ᵍʳ | ● ᵍʳ | ● ᵍʳ |
| A chart **read from a table** — orientation, labels, kind (`App::suggest_chart`) | ● ⁿ | ● | ● ᶜʰ | ● ᶜʰ | ● ᶜʰ | ● ⁵ |
| A chart's own title and legend | ● | ● | ● ᵗᶜ | ● ᶜʰ | ● ᶜʰ | ● ⁵ |
| A chart's **kind chosen** — inserted as bar, line, pie or area rather than the one the cells want, and a chart turned into another | ● ⁿ | ● | ● ᵏᵈ | ● ᵏᵈ | ● ᵏᵈ | ● ᵏᵈ |
| A **preview** of a chart before it is inserted | — | ● ᵒ | ● ᵗᵖ | ● ʷᵖ | ● ʷᵛ | ● ᵐᵖ |
| **Draws** a chart | — | ● | ◐ ᵗᶜ | ● | ● ᶜʰ | ● |
| **Draws** its title and legend | — | ● | ◐ ᵗᶜ | ● | ● ᶜʰ | ● |
| Import CSV / TSV | ● | ● ʰ | ● | ● ʰ | ● ʰ | ● ² |
| Export CSV / TSV | ● | ● ʰ | ● | ● ⁱ | ● ʰ | ● ⁴ |
| An import's seven options (`--text`, `--locale`, …) | ● | ● ʲʷ | ● ʲʷ | ● ʲʷ | ● ʲʷ | ● ʲʷ |
| Open an Excel workbook (phase 11) | ● ᵏ | ● ᵏ | ● ᵏ | ● ᵏ | ● ᵏ | ● |
| Open a CSV / TSV as a document of its own | ● ˡ | ● ˡ | ● ˡ | ● ˡ | ● ˡ | ● |
| Cell roles overlay | ● | ● | ● | ● | ● | ● |
| Name-anchor overlay | ● | ● | ● | ● | ● | ● |
| Every cell's formula at once | ● ᵍ | ● ᶠᵒ | ● ᶠᵒ | ● ᶠᵒ | ● ᶠᵒ | ● |

ᵖᵃ Palette verbs, each a `prompt()` over the core's own call — *Row height…*, *Column width…*, *Define a name for the selection…*, *Rename a name…*, *Inline a name into its uses…*, *Delete a name…*, *Evaluate a formula…*; a typed name, not a picker.
ᵖᵇ Two more buttons on each row of the *Names…* dialog — rename (an alert with an entry) and inline — over `App::rename_name` / `inline_name`, each saying how many uses it rewrote; compile- and lint-checked only.
ᵗᵘ Terminal verbs: `:filter` (no dropdowns drawn), `:rename`, `:inline`, `:value`, `:yank-values`, `:across`.
ᵉˣ The terminal's formula line reads a formula at rest in plain words (`assist::friendly_line`, the GNOME and Windows bars' own reading; `:friendly` shows it as stored) and `:explain` unfolds it in a list pane, one call a row (`friendly::explain`). The browser reads it beside the bar — the bar is an `<input>` somebody may type in, so it keeps the stored text — with Ctrl+K ▸ *Read formulas in plain words* to turn that off, and *Explain this formula in words* unfolds it in a `<dialog>`. Both smoke- or `TestBackend`-checked.
ʷᵈ Drag a column's or row's header boundary: the size follows the pointer, the two-headed cursor shows over the boundary, and release writes one `set_col_width`/`set_row_height`. The geometry (`GridGeom::edge_at`) is unit-tested; the mouse path is build-checked only — no Windows pointer was available to drive it.
ᶠᶜ Sheet ▸ Fit Column Width, measured in the cell font by GDI; from the menu, and by double-clicking the column's edge (a row's edge goes back to its content's height). Sheet ▸ Fit Row Height and Fit Content to Cells beside it.
ᶠⁱ `:fit` — the selection's columns, each as wide as its widest text in terminal cells (`unicode-width`) plus the blank between columns; `:fit rows` and `:fit all`.
ᵐᶠ Format ▸ Column ▸ Fit Width to Content, Format ▸ Row ▸ Fit Height to Content and Format ▸ Fit Content to Cells, beside the double-click on a header edge that already did the first two.
ᶜᶦ `grind sheet fit [tracks]` — columns measured in the bundled Liberation faces (`grind_print::Typesetter`), rows given back to their content; with no tracks every sheet whole. Every client's fit is `grind_sheet::fit` + `App::fit`: the shell supplies only the measuring.
ᵍᶠ The double-click on an edge, Ctrl+K ▸ *Fit Column Width* / *Fit Row Height* over the selection, and View ▸ *Fit Content to Cells* — one undo step, one per column before.
ᵍʳ `grind_sheet::chart_frame` is the one behaviour — hit-testing, drag arithmetic, minimum size, the keys and the deletion sentence — and each client is its own thin half (`doc/chart-handling.md`'s table). The CLI's column is `chart-list`/`chart-reshape`/`chart-remove` by index. The terminal's ◐ is a list rather than a pointer: `:charts` selects one, and its keys delete, nudge, scale, move and change it. The browser's is smoke-checked in jsdom, Windows' selected frame seen in a Wine render, the Mac's type-checked only; none of the three has been dragged by a hand yet.
ᵗᶜ `:charts` — every chart of the sheet drawn **in characters** (`sheet/chartview.rs`): the title, the legend where it asked for one, then a block bar per value, a sparkline per series or each slice's share. A picture of what the chart says, not of its page geometry, so ◐.
ᵗᵖ `:chart preview` draws the chart `:chart` would insert for the selection into the same pane (`verbs::preview_insert_chart`, which `insert_chart` shares its spec with) and writes nothing.
ʷᵖ Ctrl+K ▸ *Preview the chart for the selection…* — a dialog drawing it with the sheet's own `chart::svg`, **Insert** and **Cancel** (Esc); checked in `smoke.js`.
ʷᵛ Data ▸ Chart Preview… — a modal (`dialog::chart_preview`) drawing the chart through `sheet/chart.rs`'s `paint_in` (the grid's own marks), **Insert** or Cancel. Type-checked, clipped and unit-tested from Linux but **not yet seen running** — no Xvfb here.
ᵐᵖ Insert ▸ Chart Preview… — an `NSAlert` whose accessory view is an `NSImageView` of the chart rendered headless from `sheet/chart.rs`'s own `Op`s (`chart_preview.rs`), Insert / Cancel. Type-checked and linted for both Apple targets; **not yet run on a Mac**.
ᵗˢ While `:format …` is typed the status bar appends `→ <what the active cell would show>` (`App::shown_as`, nothing written); the browser's palette rows and Win32's chooser show the same beside each row.
ᵗʷ `grind-tui` with no file and neither `--sheet` nor `--text` opens `welcome.rs` — s, t, o, the arrows and Enter, with a path prompt for Open — and `:welcome` goes back from either pane (refused over unsaved work; `:welcome!` discards).
ʷᶠ Ctrl+K ▸ *Fit column width to text*, *Fit row height to content* and *Fit content to cells* — widest text in cells × an estimated 7.4px, scaled by the cell's font size (`layout::Estimate`, a `Metrics` for `grind_sheet::fit`), plus padding; estimated rather than measured because the grid's size is declared (`CELL`), rows and the whole sheet too.
ᵐᵍ Over `App::merge`/`unmerge`, `grind sheet merge [--unmerge]`): the palette and the cell menu's *Merge Cells* / *Unmerge Cells* in the GNOME window, `:merge`/`:unmerge` in the terminal, Ctrl+K's *Merge cells* / *Unmerge cells* in the browser, Sheet ▸ Merge Cells / Unmerge Cells on Windows, Edit ▸ Merge Cells / Unmerge Cells on the Mac. Every one cuts a whole row or column to the part in use first (`nav::target`). One undo step; what the covered cells held stays, out of sight. An arrow key steps over a merge and a click inside one lands on its top-left cell (`nav::through_merges`, `nav::onto_merge`) in all five. A selection *extended* across a merge is not grown to take in the whole of it — a named gap, `doc/not-doing.md` §3.
ᶜᶠ `grind sheet rule <range> <condition> --background …` adds one, `--remove <n>` and `--clear` take them away, and with no condition it lists them (`doc/conditional-format.md`). Drawing one needs nothing of a shell: `App::get_viewport` evaluates every rule and hands over the style the cell is drawn in, so every client that draws a cell's style draws the rule — the CLI's ● is `sheet view`'s styles and the list. Every window edits them too, each a face for `grind_sheet::rule`'s editing half — the eight `Look`s, `add_from_input`, `summary` — so no shell has its own idea of what a look writes or how `=B2>0` becomes ODF: the GNOME window's *Conditional Formatting…* dialog (the cells' context menu and Ctrl+K), `:rule <look> <condition>`, `:rules` and `:rule! [n]` in the terminal, three palette rows in the browser, Data ▸ Add/Remove Conditional Format… on Windows and Format ▸ Conditional Formatting on the Mac. A window offers the eight looks; the CLI's flags can still say more, and a rule written by either reads in the other.

ᶜᵇ `grind sheet checkbox <cell> --link <cell>` adds one, `--toggle` ticks it, `--remove` takes it away, and with no cell it lists them (`doc/ods-format.md` §3.5). Every shell draws the box — a GTK/GDI/AppKit square, `[x]`/`[ ]` in the terminal, a real `<input type=checkbox>` in the browser — and ticks it on a click inside the box or Space on its cell, writing the linked cell in one undo step. Making one from a window is ○ by decision for now: `doc/not-doing.md` §3.
ᵐᵈ `grind sheet merge` with no range lists them; `grind sheet view` prints the grid, and a merge's covered cells show nothing there unless they hold a value.
ᵃ In whole terminal cells and lines (`ui_tui/src/sheet/geom.rs`): a width in cells, a height
in lines of the default row's height, up to eight. Both are written back
untouched either way.
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
ᵏ `grind sheet import`, and in every window the ordinary Open (`doc/xlsx-import.md`): sniffed
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
as the core reads it, and Insert ▸ Bar/Line/Pie/Area Chart as that kind (ᵏᵈ); a right-click on a chart sets its title, its kind and its legend or removes
it, and its *Change…* takes the same words as the other windows (ᶜʰ: axes, colours, size, range).
There is no dialog.

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
| Misspelt words listed, with addresses (`doc/spelling.md`) | ● | ● | ● | ● ʷˢ | ● | ● |
| Misspelt words underlined, suggestions, Add to Dictionary | ◐ ˢᵖ | ● | ● ˢᵖ | ● ʷˢ | ● ˢᵖ | ● ˢᵖ |
| Grammar findings listed, with addresses (`doc/grammar.md`) | ● ᵍᵐ | ● | ● | ● | ● | ● |
| Grammar findings underlined, corrected from the menu, Ignore Rule | — | ● ᵍᵐ | ● ᵍᵐ | ● ᵍᵐ | ● ᵍᵐ | ● ᵍᵐ |
| **Character formatting** | | | | | | |
| Bold, italic, underline | ● | ● | ● | ● | ● | ● |
| Strikethrough | ● | ● | ● | ● | ● | ● |
| Monospace / code | ● | ● | ● | ● ᶠˢ | ● | ● |
| Colour, highlight | ● | ● | ● | ● | ● | ● |
| Font family, font size | ● | ● | ● ᶠˢ | ● ᶠˢ | ● | ● ¹ |
| Clear formatting | ● | ● | ● | ● | ● | ● |
| **Drawn**: the four booleans | — | ● | ● | ● | ● | ● |
| **Drawn**: colour, highlight | — | ● | ◐ ʰ | ● | ● | ● |
| **Drawn**: family, size | — | ● ⁱ | ○ ʰ | ● | ● ʳ | ● |
| **Block structure** | | | | | | |
| Paragraph, Heading 1–3 | ● | ● | ● | ● | ● | ● |
| Heading 4–6 | ● | ● | ● | ● | ● | ● ² |
| Title, Subtitle | ● | ● | ● | ● | ● | ● ² |
| List item | ● | ● | ● | ● | ● | ● |
| Change a list item's depth | ● | ● ⁿ | ● ᵐ | ● ⁿ | ● ᵒ | ● |
| A named paragraph style | ● | ● ᵍᵖ | ● | ● ᶻ | ● ᶻ | ● |
| **Addressing and navigation** | | | | | | |
| `p12`, `p12+40`, `#bookmark`, `§2.1.3` | ● | ● | ● | ● | ● | ● ³ |
| Outline, each row a jump | ● | ● | ● | ● | ● | ● ⁴ |
| Create a bookmark | ● | ● ᵍᵖ | ● | ● ᶻ | ● ᶻ | ● |
| Show where bookmarks anchor | ● | ● | ◐ ˡ | ● ˡ | ● | ● |
| **Pictures** | | | | | | |
| Insert an image | ● | ● | ● ᵗⁱ | ● ᵖⁱ | ● | ● |
| **Draws** an image | — | ● | ○ | ● | ● | ● |
| Alt text — a picture's short and long description | ● | ● ᵃˡ | ● ᵃˡ | ● ᵃˡ | ● ᵃˡ | ● ᵃˡ |
| **Tables** | | | | | | |
| Insert a table | ● | ● | ● | ● ᶻ | ● ᶻ | ● |
| Edit inside a cell | ● | ● | ● ᵖ | ● ᵖ | ● ᵖ | ● |
| **Draws** a table as a grid | — | ● | ● ᵠ | ● | ● ᵖ | ● |
| Merge cells, set a column width | ○ ᵗ | ○ ᵗ | ○ ᵗ | ○ ᵗ | ○ ᵗ | ○ |
| **Paper** (`doc/pdf-export.md`) | | | | | | |
| Export a PDF | ● | ● | ● | ● | ● | ● |
| Where each page begins | ● | ● | ● | — | — | — |
| Edit on the pages as they print (`doc/page-model.md`) | ● ᵖᵐ | ● ᵖᵐ | ◐ ᵖᵐ | ● ᵖᵐ | ● ᵖᵐ | ● ᵖᵐ |
| Zoom | — | ● ᵖᵐ | — | ○ | ● ᵖᵐ | ● ᵖᵐ |
| Page setup — paper, orientation, margins (fact 59) | ● | ● | ● ᵖˢ | ● ᵖˢ | ● ᵖˢ | ● ᵖˢ |
| Start a page — a page break (`doc/odt-format.md` §5c fact 58) | ● ᵖᵇ | ● ᵖᵇ | ● ᵖᵇ | ● ᵖᵇ | ● ᵖᵇ | ● ᵖᵇ |
| Print preview | ● ᵖᵖ | ● | ○ | ● | ● | ● ᵖᵖ |
| Print | — | ● | ● ˡᵖ | ● ᵖᵖ | ● ᵖᵖ | ● |
| **Files** | | | | | | |
| Open a Word document (`doc/docx-import.md`) | ● ʷᵈ | ● ʷᵈ | ● ʷᵈ | ● ʷᵈ | ● ʷᵈ | ● ʷᵈ |

ʷᵈ `grind text import`, and in every window the ordinary Open: sniffed from the bytes,
imported through `grind_docx::open`, and opened as a **new, unsaved ODF document with no path**
under `grind_docx::suggested_name` (`letter.docx` → `letter.fodt`), with `Report::summary`'s one
sentence — ᵏ's shape for the word processor. The GTK window's `.desktop` file, the Windows
associations and the Mac's Info.plist offer the type and never take it. Behind each crate's `docx`
feature, on by default.
ᵖᵐ The page model (`doc/page-model.md`). The GNOME window's view is the pages themselves — the
raster Print Preview shows, with the caret, selection and underlines drawn over it from the line
boxes the pages were set with, so a click and an arrow key land where the ink is. It zooms (Ctrl+
wheel, a pinch, Ctrl+plus/minus, Fit Width and Actual Size on the status line's zoom button) and
says the caret's page. The CLI's half is `grind text locate` and `hit`. The terminal keeps its
column of cells and rules where each printed page begins, says "page 3 of 12" on the status line
and goes to a page with `:page 3`. The Windows pane draws the pages from the same
raster and answers the caret from the same line boxes View ▸ Zoom and
Ctrl+wheel, both scroll bars, the caret's page on the status bar. The browser shows them once its
print module has loaded, the first time a text document is on screen: a canvas per
page, at the printed size — the browser's own zoom scales it. The Mac's page view is the pages too
(type-checked here): its scroll view's magnification is the zoom.
ᵖᵇ `App::break_page`, as LibreOffice writes it: the paragraph split at the caret and the second
part given an automatic style over its own with `fo:break-before`. `grind text
break p3+12` (or `p3` for a whole paragraph, `--remove` to take one away), Ctrl+Enter in the GNOME,
Windows and browser windows, `insertContainerBreak:` on the Mac, `:break` in the terminal. Backspace
at the front of the new page joins the two again, as in LibreOffice.
ᵖˢ Said in words, as `PageGeometry::set_up` reads them — `a5 landscape 15mm`, the page as it is
offered to edit: `:page-setup` in the terminal, *Page setup…* in the browser's palette, File ▸ Page
Setup… on Windows and the Mac (⇧⌘P), each an `App::set_page`. The GNOME window has a dialog
(`ui_text_gtk/src/page_setup.rs`); the Mac's native page layout panel is not used.
ᵖᵖ The CLI's preview is one page as a PNG (`grind text preview`). The Mac's is the system print
panel's own, over the PDF itself; the browser prints by opening the PDF in the browser's viewer,
and Windows, which has no API that prints a PDF, sends the preview's rasters at the printer's
resolution. Every preview anywhere is `grind_print::raster` of the display list the PDF is
written from, except the Mac's, which is the PDF. The terminal cannot show a page at all.

ˢᵖ `grind text spell --suggest` prints what each word might have been and `--add` keeps one; there is nothing to underline on a terminal's standard output. `grind-tui` underlines in red, `]`/`[` step between the words with suggestions on the status line, `:fix N` takes one and `:spell ignore|add|auto|<lang>|off` does the rest; `grind-win32` draws a Fluent-red squiggle, leads its context menu with suggestions, Ignore All and Add to Dictionary, binds F7 to the next word and has View ▸ Spelling Language…; `grind-mac` draws the dotted red underline, leads the page's context menu with suggestions, Ignore Spelling and Learn Spelling, and has Edit ▸ Spelling ▸ Check Document Now (⌘;) and Spelling Language… (type-checked only; not guessing again as a document grows is its gap, `doc/spelling.md`). The two shells share the GTK window's choice and guess-again rule through `grind_spell::{Setting, reguess}`.
ᵍᵐ **Grammar** (`doc/grammar.md`). The built-in checker (`grind-grammar`): rules for every language, English and German, run per paragraph in the language its own words show. `grind text grammar` lists findings and `grind text correct` takes one back; `grind lint` reports `grammar` and `style`; `grind grammar` remembers on or off, style, and the rules ignored. The GNOME window underlines findings (blue, dotted for style), leads the right-click menu with the message, the replacements and *Ignore Rule*, and has Grammar ▸ *Check Grammar* and *Style Suggestions*, remembered. The terminal the same, in its own words: a blue underline, `}`/`{` with the message and replacements on the status line, `:fix N`, `:grammar`. The browser draws the same lines, and its palette offers the finding at the caret (each replacement, *Ignore this rule*), *Go to the next grammar finding* and the switches, remembered in `localStorage`. Windows: the accent squiggle (dotted for style), the finding leading the context menu with its replacements and *Ignore Rule*, Ctrl+F7 to the next, View ▸ *Check Grammar* and *Grammar Style Suggestions*. The Mac: green dots, AppKit's own mark for grammar (muted for style), the finding leading the context menu, Edit ▸ Spelling ▸ *Check Grammar With Spelling*, *Grammar Style Suggestions* and *Next Grammar Finding* (⌥⌘;) — type-checked, with the drawing tested on Linux. The LanguageTool integration every client had was removed (`doc/grammar.md` §5, `doc/local-only.md`).

ᵃ Visual mode (`v`), which is the same anchor-plus-caret model under vi's spelling.
ᵇ `o` opens a paragraph below, `X` deletes the block, `:move <address>` puts it elsewhere.
ᶜ Cut/Copy/Paste over `gdk::Clipboard`, plain text both ways, a newline being a block boundary —
the same two halves `grind-web` has.
ᵈ A vi register, plain text, a newline splitting a block; `y` hands it to the system clipboard by
OSC 52 too, and a paste from the terminal goes in at the caret. `p` still puts the
register.
ᵉ Reachable only by typing the markdown notation (`~~struck~~`, `` `code` ``) — no button, no
key, no menu item. On Win32 the toggle exists in `text_emphasise` and nothing calls it with
`Emphasis::Strike` or `::Code`. `grind-text-gtk`'s format bar has a Monospace toggle, which is
the same thing under the name the document uses: `` `code` `` is a *family*, not a fifth boolean.
ʰ Sixteen colours, chosen to read on a light terminal and a dark one (`ui_tui/src/ink.rs`); one font at one size, so a code run is *dimmed* instead.
ⁱ Both, and the size reaches the line's *height* as well as its width — `metrics::size_units` is
one parse feeding the measuring attribute, `Metrics::line_height` and the drawing attribute, so a
24pt word makes room for itself. A size in a unit with no resolution here (`5cm`) is left alone.
ʳ Both: a run is measured, and drawn on the line's baseline, in its own size
(`metrics::size_px`, points or a share of its block's face), and the line is as tall as its
tallest run, as `layout::wrap` makes it everywhere. Seen in a Wine render.
ʲ Heading 4 only.
ˡ The browser draws a tick at the anchor's own offset and the name at the end of its line; the terminal writes `‹name›+offset` after the block's first line, since a mark inside the line would be a character the caret counts. Windows draws the name at the offset.
Only `grind-text-gtk` puts a tick at the exact offset — it already has `x_at` for the caret.
ᵐ `:li [depth]`. ⁿ Tab and Shift+Tab — in `grind-text-gtk`, Tab at the front of a paragraph
starts a list and Shift+Tab out of depth 1 ends it. ᵒ Ctrl+Shift+K's block-kind dialog lists four
depths; there is no Tab.
ᵖ **Free, and that is the point of the model.** A cell holds *blocks* and a block carries the
coordinate of the cell it is in (`grind_text::Cell`), so `p12` is the twelfth block whether it is
in a table or not — every caret motion, every formatting edit and every address already worked
inside a cell before any client knew tables existed. **Every client draws the grid**: the
printed pages in every graphical shell, and box-drawing rules in the terminal.
ᵠ Box-drawing rules, every column the same width. `grind_text::Faces` is handed a block's kind
and not its cell, so `grind-tui` builds a map of which block is in which cell once per frame and
reads it back — the shape `ui_text_gtk/src/view.rs`'s `Column` has, and required rather than
chosen: `Faces::of` is called while `App` holds its read lock. The map covers the view and a page
either side of it, which is the `ponytail` `doc/tui-shell.md` records.
ᶠ `:find`, then `n`/`N`, with every match marked in the line rather than only counted.
ʷ Edit ▸ Find… (Ctrl+F), Find Next/Previous (F3/Shift+F3) and Replace… (Ctrl+H) over the page: `grind_text::find` finds (ignoring case) and steps, the hit is selected, and `App::replace` writes (exactly, one undo step).
ʸ A second row in the find bar — *Replace with* and **Replace All** (`App::replace`: every occurrence, exact, one undo step). Its widget test skips without a display.
ᶻ Each a prompt over the core's own verb: the browser's palette verbs *Insert a table…*, *Bookmark this paragraph…* and *Name this paragraph's style…*, and the Windows Format menu's *Insert Table…* (Ctrl+Shift+T), *Bookmark Here…* (Ctrl+Shift+B) and *Paragraph Style Name…*. A table's size and place are `grind_text::table`'s — below the caret's block or table, never last — which the GNOME window asks too.
ᵍᵖ Move paragraph up/down and delete paragraph (`grind_text::blocks`, by caret and selection, not by address): the GNOME page's context menu, the browser's palette, the Windows Format menu. The GNOME context menu also has *Bookmark Here…* and *Paragraph Style Name…* (an alert with an entry). Insert-by-address is still the CLI's and the terminal's.
ᶜʰ Palette verbs *Insert a chart from the selection* (`verbs::insert_chart`, placed beside the table) and *Delete the last chart*. The terminal has `:chart` and `:chart!` (it writes the chart and draws none). *Change the last chart…* (and `:chart <words>`, Data ▸ Change Chart…) restyles its kind, title and legend in words (`verbs::restyle_chart`), its size too (`width=12cm height=8cm`), the cells it charts (`range=A1:C9`, read again as Insert Chart reads them), and its axes, colours and direction (`x-title="By month"`, `y-grid`, `no-x-labels`, `color1=red` — a pie's Nth slice —, `anticlockwise`), which is what GNOME's dialog sets except one bar's own colour and each series' range typed apart; *Move the last chart here* (`:chart here`, Data ▸ Move Chart Here) puts its corner at the active cell over each shell's own column and row offsets (`verbs::move_chart`). Every verb means the *selected* chart, else the last — a click selects one in the browser and on Windows, `:charts` in the terminal (ᵍʳ). Win32 has the same three as Chart-menu items and **draws** charts (`sheet/chart.rs` over `grind_sheet::chart_paint`, the Mac's marks hoisted; a Wine render checks it).

ᵏᵈ The GNOME dialog's kind buttons, one per `ChartKind::ALL`, in every other window — `verbs::insert_chart`'s `kind`, `verbs::set_chart_kind` and `ChartKind::ALL`/`name`, so the six spell them the same way. **Browser**: palette verbs *Insert a bar/line/pie/area chart from the selection* and *Make the chart a bar/line/pie/area chart*, each as a ticked row on a chart's own menu, and a button per kind over the preview that redraws it, Insert inserting the one pressed (`smoke.js` drives them). **Windows**: a **Chart** menu of its own — Insert and Change to … Chart for each kind, ticked by the kind the chart is, beside the five chart verbs that moved there from Data — the same on a chart's context menu, and a push-like radio button per kind along the preview's strip; type-checked and unit-tested from Linux, not seen running. **Mac**: Insert ▸ Bar/Line/Pie/Area Chart, and a segmented control over the preview's picture that swaps it; a chart's own menu already had the three. Type-checked, not run. **Terminal**: `:chart new bar|line|pie|area`, `:chart preview bar|line|pie|area`, and `b`/`l`/`p`/`a` in `:charts`.
ᵖⁱ *Insert a picture…* from the palette raises the file picker; the type is read from the bytes (`grind_text::picture::mime`) and `insert_below` places it — the same call GNOME, Windows and the Mac make.
ᶠˢ The browser's palette has *Font family…*, *Font size…* (a prompt each) and *Monospace*; the terminal has `:font <family>` and `:size <length>` over a selection. Both write `Change::Family`/`Size`; the terminal draws neither, being one font at one size — so its drawn-family row stays ○.
ᵃˡ `svg:title` and `svg:desc`, through one shared half — `grind_text::picture::at` (which picture the caret means), `set_alt` (trimmed, blank is none) and `advice` (the one sentence every dialog shows as it is typed, with a count against `picture::SHORT`); the CLI's is `grind text alt`. **GNOME**: an `adw::Dialog` with the picture on a card, *Alt text* and an optional long description, from the page's menu, Insert ▸ *Picture Alt Text…*, Ctrl+Alt+A and an *Add Alt Text* button on the toast an insert raises (`ui_text_gtk/src/alt.rs`; seen under Broadway). **Terminal**: `:alt <text>`, `:desc <text>`, `:alt!`; bare, either puts the command line back holding what the picture says now. **Browser**: a chip on every picture — *+ Alt text* in the accent until it has some, a quiet *ALT* after — opening a `<dialog>` with the picture, the advice and a collapsed long description, also *Alt text for the picture…* in the palette; the title is the page's own `alt` too (`smoke.js` drives it). **Windows**: Format ▸ *Picture Alt Text…* and the text pane's context menu over a picture, a ContentDialog with the picture decoded by WIC (`dialog::alt_text`, geometry `modal::alt_body`; seen under Wine). **Mac**: Format ▸ *Image Description…* — Pages' words — and the page's context menu, an `NSAlert` whose field's delegate keeps the advice current, VoiceOver named in it; type-checked, not run.
ᵗⁱ `:image <file>` puts the picture in below the caret's paragraph (`grind_text::picture`); the terminal still draws it as the placeholder character — the drawing row stays ○.
ˣ The palette is the find box, as over cells (Ctrl+F, or two characters typed after the verbs); picking a hit selects it, F3/Shift+F3 step, and Ctrl+H is *Replace in the document…*, over `grind_text::find`.
ˡᵖ `:print` hands the PDF `:pdf` would write to the system's spooler, `lp`, on its standard input, and puts its answer on the status line (checked in a pty against a stand-in `lp`).
ᵗ The model reads a `table:number-columns-spanned`, writes it back with the covered positions it
implies, projects it as `span=` and (in `grind-text-gtk`) draws it merged. Nothing **creates**
one, and no client sets a column width, because the model carries no table style
(`doc/text-core.md`).

**Mac.** ¹ Through the system font panel (⌘T), whose answer becomes a family, a size and the
weight and slant, as one undo step. ² Format ▸ Paragraph and the toolbar's
pop-up set all six levels, Title and Subtitle; ⌥⌘0–6 reach Body and the headings. ³ Edit ▸ Go To… (⌘L) asks for one; a sidebar
row or a Problems finding goes to one too, and Format ▸ Paragraph moves the selected paragraphs
up or down past their neighbour or deletes them — by where the caret is, not by address. ⁴ The sidebar's Outline section.

## 8. Where the clients differ

Everything here is reachable from the CLI but the first row, which is R9's one open exception.
Read off the ○ and ◐ cells above, largest first.

1. **Text undo is not on the CLI** (§2 ᶜ): `grind_text::Action` is not serialisable, so there is
   no text session to carry it between invocations. Every window undoes.
2. **Opening a large document, or recalculating one, blocks every window but the GNOME
   spreadsheet's.** A workbook of 1.6 million styled cells takes about a minute to open, and
   `grind-sheet-gtk` reads it on a worker thread (`Ui::load_then`: the read, `grind_open::import`
   and `grind_sheet::read_bytes` hold no `App` and no widget) and shows a toast while the window
   stays drawn. It also defers recalculation (`App::defer_recalc`) and pays it on a worker under
   a read lock, with the window's actions off. Every other shell opens and recalculates on its UI
   thread. Each needs the work off that thread, an *opening…* state and a second open refused
   meanwhile; the browser needs a Worker for it. `App::open_document` is the shared half.
3. **A table in a text document**: no client merges its cells or sets a column width (§7 ᵗ); a
   merge read from a file is kept, projected and drawn.
4. **Checkboxes in cells** are added and removed only from the CLI (§6 ᶜᵇ); every client draws and
   ticks one.
5. **Number formats beyond the presets**: every client but the CLI and the GNOME spreadsheet
   window offers decimal steps and the three currencies of `numfmt::CURRENCIES`, and no other
   symbol (§5).
6. **Blocks by address** — insert, delete, move — are the CLI's and the terminal's; the windows
   move and delete the block at the caret (§7 ᵍᵖ).
7. **The two GNOME windows have no welcome screen and do not start the other kind in place**, by
   decision: one binary per document type (`doc/suite.md`).
8. **The browser zooms a text document only by its own zoom**, which scales the pages (§7 ᵖᵐ).
9. **The terminal**, by construction: no zoom, no dragging, no pictures, one font at one size, and
   colours, borders and charts as near as character cells come (§§3–7).

## 9. Absent from every client

Not a parity problem — a feature line. Each has its row in `doc/not-doing.md` or a gate in a
shell document, and none is reachable from the CLI either.

**Spreadsheet.** Freeze panes · sort · printing · pivot tables · macros (`doc/not-doing.md` §1:
the generator is the answer, and `grind build` is the CLI's by R11). Autosave is the macOS
shell's alone, by platform convention (`doc/not-doing.md` §3).

**Word processor.** Editing a footnote or a field (both are read, shown and printed, and written
back as the file spelled them) · style *definitions* (a named style is kept and never
interpreted) · a table's own style (column widths, borders) · `table:formula`, gated on
`doc/odt-format.md` §5's open question whether a Writer table's formula is OpenFormula.

**Both.** Right-to-left layout, excluded by decision (`doc/text-layout.md` decision 1). Editing
the code view (`doc/dsl.md` §6.4). `grind build` and `grind test` are the CLI's alone by R11: no
shell links the generator.

## 10. How to re-derive this

The tables are read out of the code, not out of the shell documents, in this order:

```sh
# 1. What each application can do at all — the two authorities on the rows.
grep -oE "^\s*pub fn [a-z_0-9]+" sheet/src/lib.rs sheet/src/app/*.rs text/src/lib.rs text/src/app/*.rs

# 2. What each client reaches. Every shell has one table of verbs, and this is where it is:
sed -n '/pub const SHEET/,/^];/p' ui_web/src/command.rs   # and TEXT, below it
sed -n '/pub const MENUS/,/^];/p' ui_win32/src/menu.rs    # plus `accelerator` and `applies_to`
sed -n '/fn actions/,/^}/p'      ui_sheet_gtk/src/main.rs
sed -n '/fn actions/,/^}/p'      ui_text_gtk/src/main.rs
grep -n 'pub const HELP'         ui_tui/src/sheet/mod.rs ui_tui/src/text/mod.rs
sed -n '/pub static MENUS/,/^];/p' ui_mac/src/menu.rs      # and Command::applies, and tools.rs
grind sheet --help; grind text --help

# 3. The cross-check: a verb table can promise anything, and a shell that never calls the
#    core method cannot deliver it.
for d in ui_sheet_gtk ui_text_gtk ui_tui ui_web ui_win32 ui_mac; do
  echo "== $d"; grep -rhoE '\.(set_style|set_format|set_filter|set_col_width|import_csv)\(' $d/src | sort -u
done
```

Step 3 is the one worth keeping. A gap list is written by the person who built the shell, and
what they leave out is what they did not notice they had left out; a shell that calls neither
`set_style` nor `set_format` is four seconds of `grep`.

**Why this file is not self-checking.** The test would read each shell's verb table beside these
tables, the way `cli/tests/parity.rs` reads `sheet/src/lib.rs`. A shell's verb table is six
shapes (a `const` slice, a `Vec` of tuples, a `&str` of help text, an HTML file, a `match`, a
`static`), and normalising them costs more than it buys: this file going stale is visible the
first time somebody reads it beside a shell.
