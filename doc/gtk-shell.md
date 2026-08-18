<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The GTK shell — phase 9's native shell, planned

This is the work plan for `ui_gtk/`, phase 9's first shell, and the document that holds it
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
needs every one); each lands with its CLI command, parity row, `sample.sh` line and tests
in one change.

### C1. Addressing moves into the core (`core/src/a1.rs`)

`cli/src/a1.rs` (bracket-wrap parse, `format`, `split_range`, `sheet_dot`,
`as_definition`, `is_single`, `resolve`) moves down; the CLI re-exports, zero logic left.
The rule restates as: **the only 0↔1 conversion in the workspace is `core::a1`, the one
module every shell uses.** Free functions — no parity rows; listed in cli-parity's
"Beyond `App`" section.

### C2. Display-form formulas (`core/src/formula/display.rs`)

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

## Part II — the shell (`ui_gtk/`, crate `sheet-gtk`, binary `sheet-gtk`)

Dependencies: `sheet-core`, `libadwaita` (gtk4 reached as `libadwaita::gtk` so the two
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
  navigates, resolved through `core::a1` — the same lookup as everywhere) · formula entry
  (hexpand) · ✓/✗ buttons visible only while editing.
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
| M0 | Plan + CI prep — *done* | this document; CLAUDE.md/README rows; root `ci.yml` build job switches `--workspace` → named crates (`editor`'s system-libs trap); new `gtk.yml` (apt `libgtk-4-dev libadwaita-1-dev`, `cargo test -p sheet-gtk`, release artifact) | CI green before any shell code |
| M1 | **Read-only grid** — *done* | `ui_gtk` skeleton, window + open (argv), grid widget, `geom.rs` + tests, ScrolledWindow, theme palette, overflow/`###`, alignment | open any corpus file; smooth scroll; layout matches LibreOffice by eye (unstyled) |
| M2 | Core prep A — *done* | C1 a1-into-core, C2 display form + spans (corpus round-trip test), C3 `enter`, C4 `preview` + contract test, C5 `clear_range`, C6 `enter_range` — CLI + parity + sample.sh each | corpus display round-trip green; `sheet eval`, `set --recalc`, range `clear`, `paste` in sample.sh |
| M3 | Selection + navigation — *done* | keymap.rs Ready mode, header click/drag selection, Ctrl+arrows, status-bar aggregates | keyboard-only navigation of a corpus file feels right |
| M4 | Editing v1 + chrome — *done* | `state.rs` Enter/Edit, in-cell editor + formula bar (shared buffer), commit/cancel, undo/redo, auto-recalc + banners, Delete, sheet tabs (+ undo toast), name-box navigation, save/save-as/close-confirm | the values and formulas of `examples/sample.sh` are typeable by hand in the GUI; its formats and styles wait for M7 |
| M5 | Clipboard — *done* | copy TSV / paste TSV via `gdk::Clipboard`, cut = copy + `clear_range`, `enter_range` under it | copy in the GUI → paste into LibreOffice, and back |
| M6 | Formula UX — *done* | Point mode, F4, Tab memory, span coloring (day-one `gtk::Text` attributes spike), autocomplete (C9 lands here, core-first), signature hints, live preview | `=SUM(` + drag B2:B4 + `)` + Enter → colored, previewed, committed; one Ctrl+Z reverts edit + ripple |
| M7 | Styles + formatting UI — *done* | C7 getters (`style_at`, `format_at`, `sheet style\|format --show`) + C8 viewport styles, styled grid rendering (background, borders, weight/slant/size, both alignments, wrap), the format strip (`formatting.rs`), whose colour buttons offer `style::PALETTE` — the clrs.cc palette, in the core so `sheet style --color navy` writes the same attribute — with a dialog behind *Custom…* and *Automatic* to remove the colour | GUI- and CLI-formatted documents identical for the same operations — both build their `Format` from `numfmt::preset` and their `CellStyle` field by field, and `Format::preset_params`/`is_preset` are in the core so neither shell derives "how many decimals is this" for itself |
| M8 | Widths & heights — *done* | C10 end-to-end, `ColWidths` prefix sums in geom, header-edge drag, double-click autofit (shell measures, core stores) | a drag survives save + LibreOffice round-trip; real documents render with their true layout |
| M9 | Packaging & polish — *done* | `.desktop`, icon, AppStream metainfo, shortcuts dialog, recent files, the a11y floor, the gap list below kept true; flatpak manifest as stretch | installs and launches from a desktop environment |

M9 landed as: `ui_gtk/data/` carries the `.desktop` file, an AppStream metainfo document and a
scalable icon, none built by anything — this is a pure Cargo workspace, so there is no build
system to register them with yet; a shortcuts dialog (`gtk::ShortcutsWindow`, built from the
same `actions()` table the window wires up plus `keymap.rs`'s own vocabulary, so it cannot
list a binding the keyboard does not have); recent files, which needed no widget of its
own — `gtk::FileDialog`'s "Recent" section already reads `GtkRecentManager`, so open and save
only call `RecentManager::add_item`; and the a11y floor — `Grid::announce_active_cell`, hung
off the same `set_selection` choke-point everything else about a moved selection already goes
through, speaking the cell's address and, if it has one, its display text via
`gtk::Accessible::announce` (GTK 4.14, which is why `ui_gtk/Cargo.toml`'s `gtk4` feature moved
from `v4_12`). The flatpak manifest is the one item named "stretch" in the plan and was
skipped — nothing here needs it before the packaging files above have a build system to sit
in.

## The gaps, written down

Deferred by decision, not omission — each either has a not-doing.md row already or gets
one as its milestone lands: pointing at cells on *another* sheet (the qualified form is
written correctly, but switching sheets mid-edit is not wired) · a clipboard cell holding a
tab or a newline (they become
spaces, so the rectangle survives; the upgrade is quoting, in a codec shared with `sheet
paste`) · rich clipboard flavours, so a copy carries values and formulas as text and
nothing else · zoom (Ctrl+wheel) · freeze panes (the same-widget-headers
design accommodates them) · window-state persistence (needs a GSettings schema —
post-packaging) · autosave · a manage-names dialog (the capability exists; `sheet name`
reaches it) · merged-cell rendering (the model does not carry spans; cells render
unmerged) · full grid accessibility · typing during a background recalc · locale argument
separators. CSV, sort/filter, find/replace, the chart and print keep their existing
not-doing rows and gates.

M7 added four of its own, each named where it lives: a wrapped cell clips at its row's height
and a font size larger than the row clips for the same reason — M8 gives every row a real
height (dragged, or set with `sheet height`), but nothing yet *grows* one to fit a wrapped
cell or an oversized font, which is a layout pass the geometry has no model for
(`grid.rs`'s `draw_cells`, `font`) · a border's *line style* is ignored, so `dashed` and
`double` draw solid (`draw_borders`) · the in-cell editor draws in the widget's font rather
than the cell's, because `gtk::Text` is a real child and restyling it per cell is a second
font path · formatting a **whole column** formats its used part rather than putting a
default style on the column, which needs `table:default-cell-style-name` to become
something the model can write (`Grid::target`).

## Verification

1. `cargo test` — all suites, including the corpus display round-trip and the preview
   contract test; `cargo clippy --workspace --all-targets` clean; `reuse lint` clean.
2. Loop C green both directions, `widths` case included (needs `soffice`).
3. `SHEET=target/debug/sheet examples/sample.sh /tmp/demo` exercises every new CLI
   surface.
4. `cargo run -p sheet-gtk -- <densest sample>.fods`: open, scroll, edit, format, resize,
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
