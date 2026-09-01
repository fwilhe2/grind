<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The GTK shell — phase 9's native shell, planned

This is the work plan for `ui_sheet_gtk/`, phase 9's first shell, and the document that holds it
to the rules once building starts. It is a plan with decisions in it, not a wish list: the
open questions were closed before writing (formula syntax, recalculation policy, widths),
and each milestone names the check that ends it. The worked example is
[fwilhe2/editor](https://github.com/fwilhe2/editor)'s GTK shell and its
`doc/shared-core-native-shell.md`; where a spreadsheet diverges from a text editor — a
grid is not a `GtkTextView`, and formula entry is the whole game — the divergence is
designed here rather than discovered later.

Three decisions taken up front:

- **Formulas display and are typed in A1 display form** — `=SUM(B2:B4)` on screen, ODF
  `of:=SUM([.B2:.B4])` in the file, converted losslessly by the same lexer trick
  `cli/src/a1.rs` already uses for addresses. The argument separator stays `;`.
- **Recalculation is automatic when safe**: an edit recalculates its dependents in the
  same undo step, unless doing so would spoil a cell (a function this build lacks) — then
  the edit lands, the recalc is skipped, and a banner says why. The CLI's honesty with a
  GUI's liveness.
- **Column widths and row heights are in scope** — the model carries neither today, and a
  spreadsheet that renders every real document at uniform widths is not showing the
  document.

## The rules, applied to a GUI

The seven rules (`doc/plan.md` "Rules carried over from `editor`", `CONTRIBUTING.md`) bind
every line of this plan:

- The shell is a renderer and an event forwarder owning nothing. Selection, scroll and an
  in-progress edit are presentation state; everything else is a missing core capability.
- Reads go through `App::get_viewport`. Undo/redo lives in the core. The core pushes;
  the shell repaints from the observer, never a poll or timer.
- **Whatever the GUI can do, the CLI can do** — every capability below lands core-first,
  CLI in the same change, GUI last, and `cli/tests/parity.rs` fails the build otherwise.
- Every feature survives a LibreOffice round-trip (loop C), widths included.
- Nothing user-facing names LibreOffice (`CONTRIBUTING.md`): the file filters say
  "OpenDocument Spreadsheet".

Going straight to GTK (no TUI step) moves `editor` §3's step-3 obligation here: anything
this shell invents that a second shell would need — the 2-D scroll rule, the keymap shape,
the editing state machine, display-syntax conversion — is designed shell-agnostic from the
start: in the core, or in widget-free modules that unit-test with no display.

---

## Part I — core work first

Ten capabilities the GUI needs and the core lacks. All shell-agnostic (the wasm shell
needs every one); each lands with its CLI command, parity row, `sample-sheet.sh` line and tests
in one change.

### C1. Addressing moves into the core (`sheet/src/a1.rs`)

`cli/src/a1.rs` (bracket-wrap parse, `format`, `split_range`, `sheet_dot`,
`as_definition`, `is_single`, `resolve`) moves down; the CLI re-exports, zero logic left.
The rule restates as: **the only 0↔1 conversion in the workspace is `core::a1`, the one
module every shell uses.** Free functions — no parity rows; listed in cli-parity's
"Beyond `App`" section.

### C2. Display-form formulas (`sheet/src/formula/display.rs`)

- `to_display(canonical) -> Result<String>` — parse to AST, serialize with references
  bracketless: `[.B2]` → `B2`, `[$Data.$A$1]` → `$Data.$A$1`, `[.B:.B]` → `B:B`,
  `[.2:.2]` → `2:2`, quoted sheets stay quoted. An external-source reference stays
  bracketed — rare, unevaluated, honest. A flag on reference printing in `serialize.rs`,
  not a second serializer.
- `from_display(text) -> Result<String, DisplayError>` — a scanner (not a second parser)
  walks the text outside string literals, re-brackets reference-shaped runs, then runs the
  **existing** lexer + parser to validate and normalise. `DisplayError` carries the byte
  offset of the failure so an editor can place the caret. Disambiguation, in the module
  docs: identifier followed by `(` is a function (`LOG10(`); a bare identifier matching
  the cell pattern is a reference (`LOG10` bare is cell LOG-10 — Excel's exact collision
  and resolution), unambiguous against defined names **because `validate_name` already
  refuses cell-shaped names**; any other bare identifier is a name, untouched; bare
  `TRUE`/`FALSE` become `TRUE()`/`FALSE()`.
- `spans(text) -> Vec<(Range<usize>, TokenKind)>` — the same scanner exposed with **byte**
  ranges (Pango attribute indices are bytes), so the editor's syntax colorer and the
  committer are one code path and cannot disagree.

Tests: every corpus formula round-trips `canonical → to_display → from_display →
canonical` (loop B's harness gains a third half, same exclusion list), plus display-form
edge cases the other way. CLI: `sheet fmt --display` / `--from-display`.

### C3. `App::enter` — the typing rule, one undo step

What pressing Enter means, shared by every shell:

```rust
pub enum RecalcMode { No, Document }          // enum, not bool — dependents-only later
pub struct EnterOutcome { pub kind: Entered,  // Formula | Number | Bool | Text | Cleared
                          pub recalc: Option<Recalc> }
pub fn enter(&self, sheet: usize, pos: Pos, input: &str, recalc: RecalcMode)
    -> Result<EnterOutcome>
```

Leading `=` is a formula (canonical syntax; shells convert display form first), evaluated
and stored with its cached value. Leading `'` forces the rest as text — load-bearing, or
the strings `=x` and `123` are unenterable. Empty clears. Otherwise the literal rule
(number / TRUE / FALSE / text), moved from `cli::literal()` into the core so the CLI and
GUI cannot drift. Locale-aware literal dates: out of scope for v1, on purpose.

`RecalcMode::Document`: inside the same lock, after the edit, run the existing
`recalculated()` walk — the same check `recalc` performs, single-sourced. `spoiled == 0`:
apply the updates and push **one** undo entry `Batch([recalc⁻¹, edit⁻¹])`, so Ctrl+Z
undoes the edit and its ripple together. `spoiled > 0`: **the edit still commits**, the
recalc is skipped, the report comes back in `EnterOutcome` — refusing would make a
document using unimplemented functions read-only, which is worse than stale.

CLI: `sheet set` gains `--recalc`; `--text` maps to the `'` rule.

### C4. Formula preview — `App::preview`

`preview(sheet, pos, formula) -> Result<CellValue>`: read lock, `Engine::new(&doc).eval`,
`to_cell` — `set_formula`'s three lines minus the write. Powers the live result chip and
the status-bar aggregates. **Contract test: `preview` takes only a read lock, fires no
observer, creates no undo entry** — the shell's threading leans on all three. CLI:
`sheet eval <file> <address> '<formula>'`.

### C5. Range clear

Delete over a selection is one undo step. `clear_range(sheet, start, end) ->
Result<usize>` — `set_format`'s shape: bounded rectangle, one `Action::Batch`, skips
already-empty cells, clears formulas too. CLI: `sheet clear book.ods B2:D40`.

### C6. Range enter — clipboard's core half

`enter_range(sheet, anchor, rows: &[Vec<String>], recalc) -> Result<EnterOutcome>` — each
cell through C3's interpretation, everything plus the optional recalc in one Batch,
bounded like `set_format`. CLI: `sheet paste book.ods B2 -` (TSV on stdin).

### C7. Style and format getters

`style_at(sheet, pos)` and `format_at(sheet, pos)`. The data is one line from exposure
(`Sheet::style/format` exist; `set_style`'s docs already promise a read-first flow whose
call does not exist). GUI: bold-toggle read-merge-write, format-picker state. CLI:
`--show` on `sheet style` / `sheet format`.

### C8. Viewport carries styling

`Viewport` gains `styles: Vec<Option<CellStyle>>` + `style(row, col)`. One clone per
visible styled cell — viewport-sized, bounded (ponytail: intern when a profile says so).
Formats are not added: rendering needs the text, already carried; the picker uses
`format_at`.

### C9. Function catalog — `funcs::catalog()`

```rust
pub struct FuncInfo { pub name: &'static str, pub signature: &'static str,
                      pub brief: &'static str, pub section: &'static str }
pub fn catalog() -> &'static [FuncInfo]   // 112 entries
```

Signatures (argument names, `;`-separated — the hint UI splits on them) and one-line
briefs written from ODF 1.4 Part 4, each citing its § — the spec is the normative source,
so the clean room is not at risk. Tests: `catalog()` names exactly `implemented()`;
sections agree with `doc/small-group.md` (that parser exists in `funcs/mod.rs` tests).
Arity enforcement stays inline where it is; the catalog documents, it does not enforce.
CLI: `sheet functions --long`.

### C10. Column widths and row heights — the big one, its own milestone

- **Model**: `Sheet` gains `col_widths: BTreeMap<u32, String>` / `row_heights` — ODF's
  verbatim length strings (`"2.258cm"`), per the `style.rs` philosophy. One parser in one
  place: `style::length_mm(&str) -> Option<f64>` for renderers and loop C. Repeat runs
  store one entry per distinct width up to the used extent plus a bounded margin — a
  trailing `number-columns-repeated="16384"` must not become 16k entries.
- **Reader**: widen the style filter to keep `table-column`/`table-row` families, taking
  only `style:column-width`/`style:row-height`; read the declarations' `table:style-name`.
  `style:use-optimal-*` dropped, noted.
- **Writer**: pool distinct widths into `co{i}`/`ro{i}` automatic styles, emit
  `table:style-name` with repeats. R6: a width change sets `edits.only_values = false` —
  regenerate; splicing column elements is a later refinement.
- **Loop C**: a `widths` case, compared numerically in mm with tolerance — LibreOffice
  re-quantises lengths, like borders (doc/ods-format.md §5.4).
- **Actions/App/CLI**: `Action::SetColWidth`/`SetRowHeight`; `App::set_col_width`,
  `set_row_height`, `col_widths(sheet, Range<u32>)` + rows twin for the renderer.
  `sheet width book.ods B 2.5cm`, `sheet height book.ods 3 14pt`, `--show`.

Deliberately **not** added: a core "jump to data edge" helper for Ctrl+arrows — the shell
scans viewport-sized chunks against `used_extent`, which exists. ponytail: hoist into the
core when a second shell wants it.

---

## Part II — the shell (`ui_sheet_gtk/`, crate `grind-sheet-gtk`, binary `grind-sheet-gtk`)

Dependencies: `grind-sheet`, `libadwaita` (gtk4 reached as `libadwaita::gtk` so the two
cannot drift), `async-channel`. Imperative UI — no `.ui` files, no GResource. Pins chosen
at implementation against the ubuntu-24.04 runtime (libadwaita 1.5 baseline; the ≥1.6
accent API gets a CSS fallback). Files, each single-purpose:

| file | role | headless tests |
|---|---|---|
| `src/main.rs` | app, window, dialogs, wiring | no |
| `src/grid.rs` | the grid widget: subclass, snapshot, scrollable, editor child | no |
| `src/geom.rs` | **pure** pixel arithmetic: `cell_rect`, `hit`, `visible_range`, width prefix sums | **yes** |
| `src/keymap.rs` | **pure** key+mode → action mapping | **yes** |
| `src/state.rs` | **pure** edit-session state machine (modes, pending ref, outcomes) | **yes** |
| `src/formula_ux.rs` | autocomplete popover, signature hints, coloring wiring | no |
| `src/chrome.rs` | header bar, formula bar, sheet tabs, status bar, banners | no |
| `src/theme.rs` | palette derivation (light/dark/accent) | partly |

### Window chrome — the HIG mapping

- `adw::ApplicationWindow` + `adw::ToolbarView`; `adw::HeaderBar` with `adw::WindowTitle`
  (document name, unsaved indicator), an Open button, undo/redo symbolic buttons, primary
  menu: New · Open · Save As… · Recalculate Now (F9) · Keyboard Shortcuts · About. Ctrl+S
  saves + "Saved" toast (`adw::ToastOverlay`) — `editor`'s shape.
- **Formula bar row**: name box (narrow `gtk::Entry`; typing an address or defined name
  navigates, resolved through `core::a1` — the same lookup as everywhere; typing anything
  else *defines* it over the selection, via `App::set_name`, and the core's own refusal is
  the toast) · formula entry (hexpand) · ✓/✗ buttons visible only while editing.
- **Sheet tab strip** (bottom): linked toggle buttons + `+`; double-click / context menu
  rename popover; Delete is **immediate, with an undo toast** ("Deleted 'Q3 Actuals' —
  Undo") — the HIG undo-toast pattern, exactly what the sheet-carrying inverse was built
  for. Plain buttons, not `adw::TabBar` — wrong tool.
- **Status bar**: Sum · Count · Average of the selection via `App::preview` with generated
  formulas — `SUM`, **`COUNTA`** (a status bar's Count is non-empty, not numeric),
  `AVERAGE`; Sum/Average hidden when COUNTA is 0. Ranges clamped to `used_extent` first —
  whole-column selections must not walk a million rows. Debounced 100 ms, off-main,
  generation-counted.
- **Banners** (`adw::Banner`): after an edit that could not auto-recalc — "N formulas use
  functions this build doesn't have — recalculating would replace their saved values",
  with *Recalculate Anyway* (spoilage toasted with an Undo button); after open, if a
  background `stale()` walk finds disagreement — "Formulas are out of date" +
  *Recalculate*.
- **Dialogs**: `gtk::FileDialog` open/save-as (filters "OpenDocument Spreadsheet" `*.ods`,
  "Flat OpenDocument Spreadsheet" `*.fods`); close-request → `adw::AlertDialog`
  Save/Discard/Cancel, Save suggested and default, Discard destructive, a failed save
  cancels the close, with `editor`'s `closing` latch; `adw::AboutDialog`. App ID
  `io.github.fwilhe2.Sheet`. File on argv, consumed before
  `application.run_with_args::<&str>(&[])` (`editor`'s trick); no file → "Untitled",
  Save As.

### The grid widget (`grid.rs` + `geom.rs`)

A custom `gtk::Widget` subclass implementing `gtk::Scrollable`, inside a
`gtk::ScrolledWindow`, drawing everything in `snapshot()`. Not `GtkColumnView`: that is
row-oriented, wants to own a model (rule 1's trap in a nastier form), has no rectangular
selection, and does not virtualise 16384 columns.

- **Subclass shape**: the four Scrollable properties via `glib::Properties`
  `override_interface` (manual `ParamSpecOverride::for_interface` as fallback);
  `class_init` sets `set_css_name("sheetgrid")` and
  `set_accessible_role(gtk::AccessibleRole::Grid)`. Vfuncs: `measure` → `(0,0,-1,-1)`;
  `size_allocate` → reconfigure adjustment page sizes + allocate the editor child over the
  active cell; `snapshot` → all drawing.
- **Adjustments in pixel units** (f64 is exact far past 25M px). `upper` is **not** the
  full million-row extent: `max(used extent + one screenful, value + page_size)`, growing
  as navigation moves past it — otherwise the thumb is unusable and one click teleports
  to row 800,000. `step_increment = 3 × row_height` (wheel notch); one `configure()` per
  change under an `updating_adjustments` guard; `value-changed` → `queue_draw`
  (+ `queue_allocate` iff the editor child is visible). No custom scroll controller —
  ScrolledWindow's kinetic handling drives the adjustments.
- **Damage**: GTK4 has no partial invalidation; `queue_draw` redraws the widget, and the
  cost is bounded by visible cells (~1000 worst case). The performance lever is the
  **Pango layout cache**: `(row, col) → (text, style key, Layout)`, built with
  `create_pango_layout` (inherits font/scale/direction), kept across scroll frames,
  cleared on observer tick and `css_changed`, pruned to visible ± 1 screen.
- **Drawing order**, one pass: clip to content → base bg → per-cell style bg → selection
  fill (accent at ~0.12 α, active cell excluded) → grid lines (1-px rects) → per-cell
  clipped `append_layout` → active-cell 2-px accent border → reference outlines (editing
  only, same palette as the token coloring) → pop → headers and corner last, so scrolled
  content never bleeds over them.
- **Headers live inside the widget**, not as siblings: scroll sync is free, hit testing is
  one pure function, and freeze panes later are more clip regions calling the same
  `draw_cells(range, translate)` — sibling widgets would force the 2×2 shared-scroll
  matrix this design exists to avoid. Cost: the scrollbar track spans the header band;
  LibreOffice has the same artifact.
- **`geom.rs` is pure**: `GridGeom { header_w/h, scroll_x/y, row_height, col_widths }`,
  `ColWidths::Uniform(f64)` now and a prefix-sum vec at the widths milestone — isolating
  that enum is the entire cost of variable widths later. `hit(x,y)` returns
  Cell/RowHeader/ColHeader/Corner/ColEdge/RowEdge (4-px edge tolerance, for resize).
  Unit-tested headless.
- **Overflow/`###`**: the viewport is fetched with a ~10-column horizontal margin so
  off-screen overflow anchors still paint and neighbour-emptiness is known. Per visible
  row, one scan finds each cell's next non-empty neighbour; text overflows into empty
  neighbours, clipped at the first occupied cell — spreadsheets clip, they don't
  ellipsize; a too-narrow number draws one shared cached `"##########"` layout clipped to
  the cell. Numbers right, text left, bool/error centred, unless `CellStyle` says
  otherwise.

### Selection and keys (`keymap.rs`)

Selection is presentation state: `anchor: Pos` + `active: Pos`, plus header variants.
The keymap is `editor`'s pattern, mode-aware and pure: `action_for(key, mods, mode)`,
`gtk::EventControllerKey` on the window in Capture phase, and **`Propagation::Proceed`
for every key the keymap does not own** — that is what keeps the editor child's IME
working untouched. A Ctrl-modified key never also inserts its character, with the test.

Ready mode: arrows/Home/End/PgUp/PgDn; Ctrl+arrows jump to data edges (viewport-chunk
scans); Shift extends; Ctrl+A selects the used range; a printable char starts Enter mode
seeded with it; F2/double-click starts Edit mode; Delete → `clear_range`; Ctrl+Z /
Ctrl+Shift+Z (Ctrl+Y accepted); Ctrl+S/O/N; Ctrl+B/I via `style_at` read-merge-write;
F9 recalculates. All in the shortcuts dialog.

### Editing — one session, two views

An *edit session* owns the text, in display form throughout. The in-cell editor is a
`gtk::Text` **internal child of the grid** (allocated over the active cell, grown to the
text's natural width, capped at the viewport edge — not a `GtkOverlay`, whose
widget-relative positioning re-derives scroll math every frame); the formula bar is a
`gtk::Entry`. Both are `GtkEditable` over **one shared `gtk::EntryBuffer`** — content
syncs for free; caret and selection stay per-widget, which is precisely the familiar
behaviour (only the focused editor shows a caret). A custom-drawn editor is rejected
outright: `gtk::Text` provides IME (preedit, dead keys, CJK, Compose), caret, selection
and clipboard free, and hand-rolling `GtkIMContext` is the classic way to ship broken
input.

Commit: `from_display` → `App::enter(…, RecalcMode::Document)` → move (Enter down, Tab
right, Shift reversed; **Tab-column memory** — Enter after a run of Tabs returns to the
first Tab's column, one remembered integer, disproportionately loved). Esc cancels;
nothing touches the core. A formula that will not parse does not commit: inline error at
`DisplayError`'s byte offset, HIG error feedback, and an explicit "keep as text"
affordance (the `'` rule) — never silent mangling.

### Formula UX — the centerpiece (`state.rs` + `formula_ux.rs`)

- **Modes**, Excel's by name: *Ready → Enter | Edit*, with *Point* while the caret is
  **ref-eligible** — a pure predicate over `(text, caret)`: text starts `=` and the last
  non-space char before the caret is `= ( ; + - * / ^ & < > :`, or the caret sits in the
  current pending span. The predicate, not the mode enum, decides arrow behaviour; it
  lives in `state.rs` under a table of unit tests.
- **Transitions**, the load-bearing ones: Enter mode + arrow, ref-eligible → insert a
  pending reference, enter Point; Enter mode + arrow otherwise → commit-and-move; Edit
  mode + arrow → caret moves, never points (F2 toggles Enter↔Edit); Point +
  arrow/Shift+arrow → move/extend the pending reference, **replacing its span** in the
  buffer; grid click/drag while ref-eligible → set/extend pending (another sheet inserts
  the qualified form); typing finalises the pending span and, after an
  operator/`;`/`(`, is immediately ref-eligible again; **F4** cycles `$`-absoluteness of
  the pending ref or the ref token under the caret (found via `display::spans`).
- **Mechanism**: `Pending { span: Range<usize> /*bytes*/, anchor, active, sheet }`; every
  Point event renders the display ref and replaces `span` via `GtkEditable`
  delete/insert under an `applying_effect` guard. `state.rs` is pure —
  `on_key(mode, pending, text, caret, key, mods) -> Outcome`
  (`Passthrough | ReplaceRange | Commit{dir} | Cancel | BeginEdit{seed} |
  MoveSelection(dir) | …`); the GTK layer applies outcomes and returns
  `Propagation::Stop` only for non-Passthrough. Tested against plain strings.
- **Reference coloring**: on buffer change (guarded), `display::spans` → one
  `pango::AttrList` (byte indices align by construction), foreground per distinct
  reference from an 8-color theme-aware palette, set on both widgets
  (`Entry::set_attributes`, `Text`'s `attributes` property); the grid draws matching 2-px
  outlines, the pending ref emphasised. *Verify `gtk::Text` attribute rendering on day
  one of that milestone; the fallback if broken — color the formula bar only, keep the
  grid outlines — loses little.*
- **Autocomplete**: an identifier prefix after `=`/`(`/`;`/operator filters
  `funcs::catalog()` into a popover (name + brief), defined names below functions;
  Tab/Enter inserts `NAME(`.
- **Signature hint**: inside a call, a hint label shows the catalog signature with the
  current argument bold — argument index = `;` count at the caret's paren depth, a
  ~30-line pure function beside the state machine's tests.
- **Live preview**: debounced 150 ms (`timeout_add_local_once`, previous source
  cancelled), `App::preview` on a worker, generation-counted so stale results drop; the
  would-be result — errors included, which is half the value — in a chip at the formula
  bar's end.

### Formatting UI

A compact format strip mapping 1:1 onto the core vocabulary and nothing else: bold /
italic, text and background color, alignment, wrap (`set_style`, read-merge-write via
`style_at`); a number-format menu of exactly `numfmt::preset`'s kinds + decimals +
grouping + currency symbol + locale — the same parameters `sheet format` takes, so CLI-
and GUI-formatted documents are identical by construction. Selection-sized, one undo
step, bounded by `MAX_FORMATTED_CELLS` like the CLI.

### Recalculation policy

Every commit runs `enter(…, RecalcMode::Document)`. `spoiled > 0` → the edit lands, the
recalc is skipped, the banner explains, once per document. Manual recalc (banner button,
F9) always runs; if it spoils, a toast with an Undo button. Opening kicks a background
`stale()`; disagreement shows the gentler banner. Nothing is silently destroyed; the
common case — Small-Group-only documents — feels live.

### Threading

- Main thread: everything interactive (`get_viewport`, `enter`, undo/redo, sheet ops —
  all brief-lock).
- Worker `std::thread` with `Arc<App>`: open, save, `recalc`, `stale`, `preview`. One
  `async_channel::unbounded::<Msg>()` drained by one `glib::spawn_future_local` loop with
  a drain-to-coalesce inner loop — `editor`'s bridge verbatim: N observer ticks, one
  refetch, one `queue_draw`. The `Observer` impl just `try_send`s; handling defers to the
  next main-loop iteration, so nothing re-enters `App` during a mutation.
- **Write-lock starvation is the real hazard**: a background `recalc` holding the write
  lock blocks even `get_viewport`. Mitigation — ponytail, ceiling named: a `busy` flag
  while a background write runs; paint from the cached `Viewport`, queue no edits
  (header-bar spinner, "Calculating…"), skip preview/stale meanwhile. The upgrade is
  core-side snapshotting, not shell cleverness.
- Saves are explicit (Ctrl+S, close-confirm); R6 keeps them small-diffed. Autosave is a
  named later toggle, not v1.

### Theme, accessibility

The palette is resolved in one place (`theme.rs`): fg via `widget.color()`; accent via
`adw::StyleManager` (≥1.6) with an `@accent_bg_color` CSS fallback on 1.5; named colors
via `style_context().lookup_color` — deprecated without replacement, used and noted.
Recomputed on `css_changed` / `notify::dark` / `notify::accent-color` → clear the layout
cache, redraw. The reference palette is checked in light, dark and high-contrast. Never a
hardcoded RGBA.

The accessibility floor, stated honestly: `AccessibleRole::Grid` plus an accessible
description announcing the active cell on move. A custom-drawn grid is otherwise
invisible to assistive technology; full grid a11y is a future milestone of its own,
recorded here rather than pretended.

---

## Part III — milestones

Ordered on one insight: **the read-only grid is the highest-risk item and needs zero new
core API** — `get_viewport` exists — so it goes first, de-risking the custom widget while
core work proceeds. Every milestone lands green: `cargo test`, clippy clean, `reuse
lint`, the loops, parity.

| # | Milestone | Contents | Exit criterion |
|---|---|---|---|
| M0 | Plan + CI prep — *done* | this document; CLAUDE.md/README rows; root `ci.yml` build job switches `--workspace` → named crates (`editor`'s system-libs trap); new `gtk.yml` (apt `libgtk-4-dev libadwaita-1-dev`, `cargo test -p grind-sheet-gtk`, release artifact) | CI green before any shell code |
| M1 | **Read-only grid** — *done* | `ui_sheet_gtk` skeleton, window + open (argv), grid widget, `geom.rs` + tests, ScrolledWindow, theme palette, overflow/`###`, alignment | open any corpus file; smooth scroll; layout matches LibreOffice by eye (unstyled) |
| M2 | Core prep A — *done* | C1 a1-into-core, C2 display form + spans (corpus round-trip test), C3 `enter`, C4 `preview` + contract test, C5 `clear_range`, C6 `enter_range` — CLI + parity + sample-sheet.sh each | corpus display round-trip green; `sheet eval`, `set --recalc`, range `clear`, `paste` in sample-sheet.sh |
| M3 | Selection + navigation — *done* | keymap.rs Ready mode, header click/drag selection, Ctrl+arrows, status-bar aggregates | keyboard-only navigation of a corpus file feels right |
| M4 | Editing v1 + chrome — *done* | `state.rs` Enter/Edit, in-cell editor + formula bar (shared buffer), commit/cancel, undo/redo, auto-recalc + banners, Delete, sheet tabs (+ undo toast), name-box navigation, save/save-as/close-confirm | the values and formulas of `examples/sample-sheet.sh` are typeable by hand in the GUI; its formats and styles wait for M7 |
| M5 | Clipboard — *done* | copy TSV / paste TSV via `gdk::Clipboard`, cut = copy + `clear_range`, `enter_range` under it | copy in the GUI → paste into LibreOffice, and back |
| M6 | Formula UX — *done* | Point mode, F4, Tab memory, span coloring (day-one `gtk::Text` attributes spike), autocomplete (C9 lands here, core-first), signature hints, live preview | `=SUM(` + drag B2:B4 + `)` + Enter → colored, previewed, committed; one Ctrl+Z reverts edit + ripple |
| M7 | Styles + formatting UI — *done* | C7 getters (`style_at`, `format_at`, `sheet style\|format --show`) + C8 viewport styles, styled grid rendering (background, borders, weight/slant/size, both alignments, wrap), the format strip (`formatting.rs`), whose colour buttons offer `style::PALETTE` — the clrs.cc palette, in the core so `sheet style --color navy` writes the same attribute — with a dialog behind *Custom…* and *Automatic* to remove the colour | GUI- and CLI-formatted documents identical for the same operations — both build their `Format` from `numfmt::preset` and their `CellStyle` field by field, and `Format::preset_params`/`is_preset` are in the core so neither shell derives "how many decimals is this" for itself |
| M8 | Widths & heights — *done* | C10 end-to-end, `ColWidths` prefix sums in geom, header-edge drag, double-click autofit (shell measures, core stores) | a drag survives save + LibreOffice round-trip; real documents render with their true layout |
| M9 | Packaging & polish — *done* | `.desktop`, icon, AppStream metainfo, shortcuts dialog, recent files, the a11y floor, the gap list below kept true; flatpak manifest as stretch | installs and launches from a desktop environment |
| M10 | Row auto-height & zoom — *done* | a row without a height of its own is measured from what is in it; Ctrl+wheel and Ctrl+`+`/`-`/`0` scale the view; the in-cell editor draws in the cell's font | a wrapped cell and a 28pt cell are drawn whole; the grid at 1.6× is the same grid, bigger, editor included |

M9 landed as: `ui_sheet_gtk/data/` carries the `.desktop` file, an AppStream metainfo document and a
scalable icon, none built by anything — this is a pure Cargo workspace, so there is no build
system to register them with yet; a shortcuts dialog (`gtk::ShortcutsWindow`, built from the
same `actions()` table the window wires up plus `keymap.rs`'s own vocabulary, so it cannot
list a binding the keyboard does not have); recent files, which needed no widget of its
own — `gtk::FileDialog`'s "Recent" section already reads `GtkRecentManager`, so open and save
only call `RecentManager::add_item`; and the a11y floor — `Grid::announce_active_cell`, hung
off the same `set_selection` choke-point everything else about a moved selection already goes
through, speaking the cell's address and, if it has one, its display text via
`gtk::Accessible::announce` (GTK 4.14, which is why `ui_sheet_gtk/Cargo.toml`'s `gtk4` feature moved
from `v4_12`). The flatpak manifest is the one item named "stretch" in the plan and was
skipped — nothing here needs it before the packaging files above have a build system to sit
in.

M10 closed two of M7's four gaps, and both fell out of the geometry rather than being drawn
around. **A row is fitted to what is in it** unless the document gave it a height of its own,
which still wins and still clips — `Grid::measure_rows` lays out only the cells that *can*
overflow a default row (one that wraps, or one whose style asks for a bigger font; a cell
with no style of its own is skipped without being laid out), and the result is handed to
`Sizes` beside the document's own heights. It is measured once per document change rather
than per frame, because a row above the view displaces the ones below it and the pass
therefore cannot be limited to what is on screen; `AUTO_HEIGHT_CELLS` is the ceiling on how
much sheet is measured at all, past which every row keeps the default height. **Zoom** is one
factor in `Grid::geom`: `Sizes::scaled` for both axes, the metrics and the header band
multiplied, and the font scaled by a Pango *scale* attribute so a cell that set its own size
zooms with everything else. Nothing measured is stored zoomed — a natural row height, an
autofit width and a resize drag all become document lengths at 1×, which is what keeps a
document saved at 200% identical to the same document saved at 100%.

It also closed M7's third gap on the way, because zoom made it visible: **the in-cell editor
draws in the cell's font**. It is a real `gtk::Text` child rather than something this widget
draws, so it is told what the cell looks like instead of inheriting it — as Pango attributes,
not CSS, since attributes are what the grid uses for the same cell and what the reference
colouring already speaks, and the two therefore merge into one list. The font only: weight,
slant, size and the zoom. The *colour* stays the theme's, because the reference colouring owns
the foreground while a formula is being typed and a cell colour underneath it would be a
second opinion about the same bytes. The cell's padding scales with everything else.

After M10, two polish passes changed what the plan above says about the chrome, and this
paragraph is the record. **Motion**: zoom anchors on the pointer (Ctrl+wheel), a pinch
gesture, or the view centre (keyboard), with a status-bar readout away from 100%; long
programmatic jumps glide ~150ms (off with the system's animation setting) and land with a
track of context; a resize drag shows the length it is choosing; the active cell and
reference outlines have softened corners; selected header labels are bold and accented; the
palette follows dark/accent changes at runtime via `adw::StyleManager` notifications (GTK
does not expose the `css_changed` vfunc); the routine "Saved" toast is gone — the subtitle
clearing is the confirmation. **Chrome**: the format strip became the Format page of a
mode-switched tool row (`chrome::tools`) — Format · Calculate · View, plain linked toggles
over a `gtk::Stack` (`AdwToggleGroup` is 1.7; the pin is 1.5), where Calculate holds
Recalculate · Explain · Calculations… · Names… · Copy Value and
View holds the zoom group, Fit Content
and the Friendly Formulas toggle. The mode never switches itself: auto-appearing contextual
tabs are the one ribbon behaviour deliberately not copied, being both the legally
distinctive part of that design and the HIG's least favourite. Every button activates a
`win.` action the window already owns, so the strip adds reachability, never capability.
The primary menu slimmed to files · sheets · Keyboard Shortcuts · About, which is the HIG's
idea of one. *(That tool row is gone: "Four surfaces" below replaced it, and is what this
window does now. The paragraph stays because the section that replaced it is an argument
against this design, and an argument with the losing side deleted is not one.)*
**Fill**: the Fill Down / Fill Right buttons then left that row again, replaced
by the convention every other spreadsheet has — a handle on the selection's bottom-right
corner (`geom::GridGeom::fill_handle`), dragged in any of the four directions, outlining
where it is pointing until the pointer is released (`keymap::fill_target` /
`keymap::fill_rect`, both pure and tested). The source is the selection's edge facing the
drag, which is what makes dragging *up* or *left* mean anything. Ctrl+D / Ctrl+R and the
`win.fill-down` / `win.fill-right` actions behind them are untouched, so nothing became
mouse-only.

## Four surfaces — normative for this window's chrome

The mode-switched tool row above lasted exactly as long as it took to add three more features,
which is the observation this section exists to answer: **more features kept becoming more
buttons.** The switch was the right instinct and it delayed the problem rather than solving it.

### What was actually wrong

The three pages were three *different kinds of control* wearing one presentation:

| Page | What it really was | Does it grow? |
|---|---|---|
| Format | A **property inspector** — every control a two-way binding to `CellStyle` / `numfmt::Format` on the selection | No: those are closed vocabularies |
| View | **View state** — `doc/view-modes.md`'s readings, which write nothing at all | No: there are three readings and a zoom |
| Calculate | A **list of verbs**, and the only page with no membership rule | Without bound |

*Filter* and *Copy Value* are not calculations; they were on that page because it was the page
things went. And since **every new feature is a verb**, every new feature landed there. The
toolbar was not growing because nobody was watching it. It was growing because verbs had
nowhere else to go, and one of the three tabs had an admission test that could not refuse
anything.

So the fix is not a fourth page, a smaller icon or a stricter reviewer. It is to give verbs a
home that is *supposed* to grow, and to give every other surface an admission test that closes it.

### The rule

Four surfaces. Each has a test that says what may go in it, and only one of them grows.

| Surface | A control belongs here when… | Grows with features? |
|---|---|---|
| **Header bar** | it is about the document as a whole, or about the window | **No** — five slots, and they are spoken for |
| **Format bar** (`chrome::format_bar`) | it **reads *and* writes** a property of the selection | No — bounded by `CellStyle` + `Format` |
| **Context menus** (cells, sheet tab, column/row header, chart) | it acts on the thing under the pointer | Slowly, bounded by what the thing *is* |
| **Command palette** (`palette.rs`) + the menus | anything else | **Yes. This is the growth valve** |

Stated as one sentence, which is the sentence to hold this window to:

> **A new feature is a verb. A verb gets a row in a table. It does not get a button.**

### Where everything went

- **Header bar**: Open · Undo/Redo · | · Find a Command (Ctrl+K) · View · ☰. The *Chart*
  button left it — inserting a chart is a verb about a selection, so it is in the cell menu and
  the palette. Nothing is added to this bar again.
- **Format bar**: exactly the old Format page, unwrapped from the stack. It is now the window's
  only toolbar and its membership rule is the whole reason it can stay visible.
- **View menu** (`chrome::view_menu`): the zoom group as a custom child, Fit Content, and the
  three readings plus Friendly Formulas as check items over their stateful actions. A menu
  rather than a row of toggles because none of it is about the selection; the check marks are
  the actions' own state, so there is nothing to keep in step.
- **Cell context menu** (`grid::cell_menu_model`): Cut · Copy · Copy Value · Paste — Clear
  Contents · Fill Down · Fill Right — Name This Range… · Filter Rows · Insert Chart…. This
  window had no context menu on its *content* at all, which was a plain HIG gap, and it is
  where the Calculate page's selection verbs always belonged. A right-click outside the
  selection moves the selection there first; a right-click during an edit stores it, exactly as
  a left-click elsewhere does.
- **Sheet tab context menu** (`chrome::tab_menu_model`): Rename… · Delete, on the tab, which is
  the only spelling that says *which* sheet.
- **Primary menu**: the four file verbs, the four things done to a document as a whole
  (Recalculate, Check the Document, Find a Calculation…, Names…), and Find a Command ·
  Keyboard Shortcuts · About. **Nothing about the selection**, which is the HIG's own rule for
  a primary menu and the thing that finally sizes it.
- **Command palette**: Ctrl+K, the search button, or the menu. Type three letters, Enter.

Four `win.` actions were added so the context menu had something to point at — `copy`, `cut`,
`paste`, `clear`, which the keyboard could already reach and no menu could. They deliberately
carry no accelerator: `keymap.rs` owns Ctrl+C/X/V and Delete inside the grid, and a second
binding for one key is how a shortcut ends up doing two things.

### Why this cannot rot

The palette is **a view over `main.rs`'s `actions()` table**, which is where a `win.` action,
its accelerator, its menu entries and its shortcuts-window row already came from. So the growth
rule is structural, not a habit: there is no way to add a verb to this window without it
appearing in the palette, because there is no second place to add one. `shortcuts()` lost its
private list of titles in the same change and reads that table too — a verb spelled one way in
the palette and another in the shortcuts window is a verb a reader has to learn twice.

`main.rs`'s `chrome_tests` are the ratchet, in the shape `cli/tests/parity.rs` established:

- every verb is in the palette, or is the one named exception (the palette cannot offer to open
  itself);
- every `win.` action named by **any** of the four menu models exists — which is why each menu
  is a function returning a `gio::Menu` rather than one built inline, and why `gio::Menu` being
  GIO rather than GTK matters: the walk needs no display. A menu item naming an action nobody
  declares is otherwise *silent*, drawn and greyed out as though the feature were merely
  unavailable;
- a verb lives in one menu, not two, with the one genuine exception listed rather than allowed
  by a rule (`win.names` means "name this range" from the cells and "manage the names" from the
  document menu);
- no two verbs share a name, and a reading is never also a plain verb — `add_action` would
  replace the stateful one with a plain one and the check marks would stop working.

`palette::rank` and `keys` are pure and tested with no display; the ranking itself is
`grind_core::search::score`, moved down from `ui_web/src/command.rs` so that the two shells with
a palette cannot rank the same query two ways. That is the only thing this window and the
browser one now share, and it is the right amount: the *vocabulary* stays per-shell, because
`ui_web`'s list is its own answer to having no menu bar at all (`doc/web-shell.md`).

### What this is not, and the ribbon question

Two deliberate refusals, both recorded because they will look like omissions:

- **The palette is not a go-to box.** `grind-web`'s also jumps to an address, a sheet or a
  defined name, because a browser tab has nowhere else to put one. This window has the formula
  bar's name box, which is where a spreadsheet's user already looks, and two boxes that both
  take `B12` is one too many.
- **The palette is not a launcher for what a pointer wants.** Bold stays on the format bar
  because the format bar *shows whether the cell is bold*. The palette can only run it.

On the ribbon: the tab strip is gone, so the one element of this window that resembled that
design at all is no longer there — which settles the question by construction rather than by
argument, and is a nice side effect of fixing the real problem. The elements that are
distinctive to that design and were never copied, before or after: **contextual tabs that
appear and disappear with the selection** (chrome that moves under the pointer, and the HIG's
least favourite thing too), a large application button opening a file panel, galleries that
mutate the document on hover, and oversized multi-line tooltips. What replaced the strip is a
fixed property bar, plain popover menus, and a search box — the last of these having prior art
running from `M-x` and `:` through every editor since. None of this is a legal opinion; it is
the design rule, and the design rule is: diverge, rather than rely on anything having expired.

## The gaps, written down

**The code view is read-only** (`doc/dsl.md` §6, D9), and is the one thing in this window that
arrived after M10. Ctrl+Shift+U, *Show the Source* in the View menu, or the palette:
the document's projection on the other page of a `gtk::Stack`, with the active cell's own line
marked and moving the cursor in it selecting the cell that line projects. A stack rather than a
paned split — §6.2 is right that a split is what a person eventually wants, and it is also a
second viewport to keep in step. Editing it is gated in §6.4.

**Check Document** — `grind lint`'s findings, as a list (`doc/dsl.md` §4.3, D6). F8, the
primary menu, or the palette: a dialog of rows, each an icon, a message, the
address it is about and the rule id that said it — the id because that is the word
`grind sheet lint --off <rule>` takes, and a diagnostic nobody can name is one nobody can
silence. Activating a row selects the cell and closes; the *Hints* toggle is `--hints`, off by
default here as everywhere. A dialog rather than a docked panel for `Calculations`' reason: this
list is consulted, acted on and closed, and `lint.rs` says so. **Nothing about the rules is in
this shell** — a diagnostic arrives with its own address, and jumping to one is `a1`'s job.

It also carries the one bug this window has had since M10, written down because the shape of it
recurs: **a handler that runs because the cursor moved must not move the cursor.** Marking the
current line by placing the cursor on it made GTK deliver `notify::cursor-position` again — not
inside the call, so the "I am updating" latch never saw the re-entry — and the window stopped
answering. `code::mark` now applies the tag and nothing else, `code::go_to` is the half that
drives the view, and the handler is idempotent per line so any scheduling of the signal is safe.
`ui_text_gtk`'s widget test asserts that `mark` moves nothing.

Deferred by decision, not omission — each either has a not-doing.md row already or gets
one as its milestone lands: pointing at cells on *another* sheet (the qualified form is
written correctly, but switching sheets mid-edit is not wired) · a clipboard cell holding a
tab or a newline (they become
spaces, so the rectangle survives; the upgrade is quoting, in a codec shared with `sheet
paste`) · rich clipboard flavours, so a copy carries values and formulas as text and
nothing else · freeze panes (the same-widget-headers
design accommodates them) · window-state persistence (needs a GSettings schema —
post-packaging) · autosave · a manage-names dialog (the capability exists; `sheet name`
reaches it) · merged-cell rendering (the model does not carry spans; cells render
unmerged) · full grid accessibility · typing during a background recalc · locale argument
separators · **moving over a filtered-out or manually hidden row**: the arrow keys still
step onto a row that has no height, so the selection appears to stick until it passes the
run (`keymap.rs` is pure and knows nothing about the document, so skipping them means
handing it the hidden set — worth doing the first time it annoys somebody, not before).
CSV, sort, find/replace and print keep their existing not-doing rows and gates. The chart's
own gaps moved again: creating one from the GUI, *editing* one (double-click, or right-click
→ Edit Chart…), deleting one from that same menu, assigning a colour by hand, and every part
of an axis — its title, its tick labels, its gridlines — are now built
(`doc/chart-format.md`). What remains of that row is the keyboard: no keyboard-driven
repositioning, and no way to reach a chart's dialog without a pointer at all.

**Filtering is built** (§9.4): the dropdown button in each heading cell of the range, a
value list behind it (`filter_ui.rs`), and `win.filter` / Ctrl+Shift+L — reached from the cell
menu, since a filter is about a selection — to put a filter over the selection or clear it. Which rows that hides comes from the core and
is never stored (`sheet/src/filter.rs`), so the grid asks `App::hidden_rows` per paint and
draws those rows at zero height.

**Hiding rows and columns by hand is built** (§5.4, `table:visibility="collapse"`) — the
persisted twin of the filter above, and orthogonal to it: `Sheet::hidden_cols`/
`Sheet::row_manually_hidden` in the core, `App::set_col_hidden`/`set_row_hidden`/
`hidden_cols`/`manually_hidden_rows`, and `sheet hide`/`--unhide` on the CLI. Right-clicking
a column or row header (`Grid`'s one context menu, `ui_sheet_gtk/src/grid.rs`) hides it, or the
whole run under a header selection. `Grid::col_sizes`/`geom` fold the hidden set into
`Sizes` the same zero-width-track trick the filter already used for rows, so a hidden
column or row draws nothing and displaces nothing. The one thing filtering did not need: a
hidden run collapses its two neighbours' headers together, so `geom::Hit` gained
`HiddenCols`/`HiddenRows` for the marker standing at that collapsed boundary
(`Sizes::hidden_run`, tested headless in `geom.rs` — no display needed to prove a click
there finds the right run from either side) — a thin accent bar, clickable to unhide the
whole run in one step.

**Inline name hints are built** (`doc/view-modes.md` V4, normative there): View → *Show Where
Names Live*, or
Ctrl+Shift+N, draws each named expression inside the cell it is bound to, once per anchor, with a
range outlined; and the formula bar gains a third stack page reading a formula through its names
(§3.3). Both are *readings* — the file is byte-identical with the mode on, which is what lets
them sit on a toolbar with no confirmation, no dirty flag and no undo entry. The arithmetic is in
`geom.rs` (`hint_rect`, `GridGeom::hint_cell`) and tests with no display.

**Role mode is built too** (V6): View → *Show What Each Cell Is*, or Ctrl+Shift+R, colours every
cell's text by what
it is and suppresses the document's own colours while it is on — §4.5's decision, with a hairline
under any cell whose styling is being hidden and a **Roles** button in the status bar, since the
mode has to say it is on somewhere always visible. Its accessibility floor shipped in the same
commit rather than after it (§4.6): one distinct glyph per role at each cell's leading edge, a
corner triangle for the roles that are also diagnostics, and `Accessible::announce` speaking the
role and the name on every selection move. `theme::role_color` takes the hues from
`style::PALETTE` and walks each towards the theme's foreground until it separates from the
theme's background, so the mode reads on a dark sheet without a second table of colours;
`--render-to --overlay names|roles|all` renders a frame with the modes on.

M7 added four of its own, each named where it lives — three of them closed by M10 above.
What is left: a border's *line style* is ignored, so `dashed` and
`double` draw solid (`draw_borders`) · formatting a **whole column** formats its used part rather than putting a
default style on the column, which needs `table:default-cell-style-name` to become
something the model can write (`Grid::target`).

## Verification

1. `cargo test` — all suites, including the corpus display round-trip and the preview
   contract test; `cargo clippy --workspace --all-targets` clean; `reuse lint` clean.
2. Loop C green both directions, `widths` case included (needs `soffice`).
3. `SHEET=target/debug/sheet examples/sample-sheet.sh /tmp/demo` exercises every new CLI
   surface.
4. `cargo run -p grind-sheet-gtk -- <densest sample>.fods`: open, scroll, edit, format, resize,
   save; reopen the saved file in LibreOffice — identical display.
   `--render-to <png>` after it draws one frame and exits, which is how a custom-drawn
   widget gets an output a machine can keep — `editor` §5's rule that every boundary wants
   a program exercising it where the UI cannot go. Not a user feature and not in the menus.
5. M6's script by hand: `=SUM(` + drag + `)` + Enter; one Ctrl+Z reverts edit and ripple;
   the spoilage banner appears on a document using non-Small-Group functions and never
   otherwise.

## Risks, honestly

1. `gtk::Text` Pango-attribute rendering fidelity — day-one spike in M6; the fallback
   (formula bar only + grid outlines) is defined and loses little.
2. `StyleContext::lookup_color` is deprecated without a replacement; the adw accent API
   is ≥1.6 — the CSS-named-color fallback covers 1.5.
3. RwLock write starvation during a background recalc — the busy/cached-viewport policy
   is the mitigation, and its ceiling is named.
4. `glib::Properties` `override_interface` syntax churns between gtk4-rs versions — the
   manual `ParamSpecOverride` path is the safety net.
5. Whole-range extent iteration in the engine (aggregates, whole-column references) —
   the shell clamps to `used_extent`; sparse range iteration in the engine is the
   eventual core upgrade.
