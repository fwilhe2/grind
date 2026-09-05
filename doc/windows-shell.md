<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The Windows shell

The plan for `ui_win32/` — crate `grind-win32`, binary `grind-win32.exe` — and the decisions
behind it. Normative for that directory the way `doc/tui-shell.md` is for `ui_tui/`,
`doc/web-shell.md` for `ui_web/` and `doc/sheet-shell.md` for the spreadsheet's GTK window.

**Built through W3; W4–W8 are a plan.** This document was W0's deliverable — the shell planned,
its decisions argued, its gaps named in advance, and the two claims the whole thing rests on
measured rather than assumed. W1 added the window and the read-only grid, W2 the selection and
W3 the editing, so the parts of it that were predictions are now records: what is written down
about the geometry, the theme, the double buffer, the scrollbars, **decision 5's windowless
render target**, **decision 4's menu bar** and **decision 7's modals** has been run.

## In one line

**A Win32 window over `grind-core`, hosting both document types, whose `.exe` depends on
nothing Windows does not already ship.** No .NET, no WinUI, no Windows App SDK, no Visual C++
redistributable — it runs on a clean Windows install and it runs under Wine.

## The precedent

`fwilhe2/editor`'s `doc/decision-win32-shell.md` (accepted 2026-09-04) replaced a WinUI 3 shell
in C# reaching the core through UniFFI with `ui_win32/`: a plain Win32 window drawn with GDI
through Microsoft's own [`windows`](https://crates.io/crates/windows) crate. Its argument
transfers here wholesale and is not re-litigated in this document. The short form:

- Building the C# shell needed four separate installations and a 174-line PowerShell installer;
  *running* it needed two runtimes that are not part of Windows.
- The `uniffi` ↔ `uniffi-bindgen-cs` pin was that repository's most fragile version coupling.
- Nothing about the shell could be examined from a Linux development machine — not built, not
  type-checked, not linted.
- Following a platform's *conventions* and using a platform's *controls* are not the same
  thing. What "native" buys a user is the shell font at the right size, their own wheel-scroll
  and caret-blink settings, Ctrl+Y for redo, a Save/Don't Save/Cancel dialog with Save as the
  default, a dark title bar when the theme is dark, per-monitor DPI that reflows on the drag
  between monitors. A Win32 window has every one of those. What is genuinely lost is the *look*
  of Fluent, which is narrower than "nativeness".

What does **not** transfer is the size of the job. That shell is 1733 lines drawing one
monospaced text buffer, and the entire surface it needed from a text stack was two numbers.
This one has a grid with per-column widths, cell styles, number-formatted display strings, and
a word processor whose line breaking lives in `grind-core` and asks the shell for **cumulative
advances of arbitrary styled text**. The measurement question is therefore genuinely open here
where it was settled there, and it is decision 3 below.

## What the core already gives

The reason this is a shell and not a project: **it needs no new core API.** Everything below
exists today and is exercised by at least two other shells.

| Need | Reached by |
|---|---|
| Which document type some bytes are | `grind_core::kind` — sniffed from content, never the file name |
| A rectangle of cells, with texts, styles and overlays | `grind_sheet::App::get_viewport` / `get_viewport_with` |
| Column widths and row heights, as ODF lengths | `App::col_widths` / `row_heights`, parsed with `grind_sheet::style::length_mm` |
| Typing a value or a formula, one undo step | `App::enter`, `enter_range`, `clear_range`, `preview` |
| Line breaking, and every caret motion defined in terms of a line | `grind_core::layout` through `grind_text::App::layout_block` / `caret_x` / `caret_line` / `caret_line_bounds`, given a `Metrics` and a `Faces` |
| Direct character formatting over a span | `App::char_style` / `set_char_style` |
| `**bold**` as it is typed | `App::type_markdown` — in the core so four shells cannot read `**` four ways |
| The document as its projection, and the four line-shaped questions a code view asks | `App::project`, `Projection::line_count` / `line_span` / `line_pieces` / `address_on_line` |
| What the document says about itself | `App::lint` → `grind_core::lint::Report` |
| What each cell *is*, and where a name anchors | `view::Overlays`, `CellRole::marker`, `NameAnchor` |
| The colour list a shell offers | `grind_core::style::PALETTE` |
| Repaint on change | `grind_core::Observer` — the core pushes, shells never poll |

The one thing it wants and the core does not have yet is **L3**: `ui_sheet_gtk`'s row
auto-height measurement moving onto `layout::Metrics`, so one breaker serves both applications.
Until that lands, this shell honours the row heights a document *stores* and does not measure
its own — a named gap below, not a surprise.

`grind_core::search::score` is deliberately **unused** here. See decision 4.

## The decisions

### 1. One binary, both document types

`grind-win32.exe` opens a spreadsheet or a text document, chosen by `grind_core::kind` reading
the file's *bytes*. `--sheet` / `--text` answer only the empty case, and asking for one when the
file is the other is an error rather than a silent override — `ui_tui/src/main.rs` already has
this exact shape and it is copied, argument parsing included.

This follows `grind-tui` and `grind-web` rather than the two GTK binaries, and the reason the
GTK shells split does not exist here. `grind-sheet-gtk` and `grind-text-gtk` are separate
processes with separate app IDs **because a `.desktop` file's `MimeType=` is per application**.
Windows associates files through per-ProgID registry keys, and one executable registers as many
ProgIDs as it likes — each with its own icon, description and verbs, all pointing at
`grind-win32.exe "%1"`. The platform's own mechanism is the reason, in both directions.

The file name is a build artifact; the **display name is "Grind"** — window title, Start-menu
entry, About box, and the AppUserModelID that decides taskbar grouping.

### 2. Win32 + GDI, through the `windows` crate

No manifest, no COM apartment beyond what the file dialog needs, and `+crt-static` in a new
`.cargo/config.toml` so the MSVC C runtime is linked in rather than looked for. The claim in
the one-liner above is then *checkable*, and CI checks it by reading the import table back
rather than trusting it.

This is already measured, before a line of the shell exists — see *Evidence* at the end.
`grind.exe`, built for `x86_64-pc-windows-msvc` from this workspace on this Linux machine,
imports exactly four DLLs, all of them part of Windows.

Rejected, each for the reason `doc/decision-win32-shell.md` gives at length: WinUI 3 in C#
(two runtime installs), `windows-reactor` (0.x, four months old, still needs the App SDK
runtime, and its hooks put state in the shell — which every shell here is forbidden), XAML
Islands (deprecated, fussy unpackaged), and **an `EDIT` or rich-edit control for the document**,
which is rejected on architecture rather than on effort: it owns its own text buffer, exactly as
`GtkTextView`, WinUI's `TextBox` and `contenteditable` do, and the moment one exists there are
two sources of truth.

That last rejection has a boundary worth stating, because the shell walks right up to it: the
**formula bar and the in-cell editor are a child `EDIT` control**, and that is fine. It holds
the in-progress *input*, not the document; committing goes through `App::enter`. `ui_sheet_gtk`
draws the same line with a `gtk::Entry`. A control that holds a keystroke is a widget; a control
that holds the document is a second model.

### 3. How text gets measured — the one genuinely open question

`doc/text-layout.md` closed on Path C: line breaking lives in `grind-core`, and a shell supplies
only the font. The trait is one method that matters —

```rust
fn advances(&self, text: &str, style: &TextStyle, out: &mut Vec<f32>);
```

— exactly one cumulative advance per `char`, in whatever unit the shell answers in. Never
negative, never decreasing.

**The other three shells were each handed the answer by their toolkit**, which is why none of
them had to make this decision. The terminal counts cells (`unicode-width`, twenty lines). The
browser asks the browser. GTK asks Pango — `line.index_to_x(byte, trailing)` in
`ui_text_gtk/src/metrics.rs`, which is **cluster-aware**: `e` + U+0301 is one cluster, so both
characters get the same x and the combining mark contributes no width. Correct by construction,
because Pango is a shaping engine.

**Win32 has no Pango.** GDI does glyph mapping, not shaping, and that is the whole of the
question.

#### The decision is not "which measuring API"

`GetTextExtentExPointW`'s `lpnDx` output *is* the array this trait asks for — the width of the
string up to and including each unit. Two things follow from it and only one is cheap:

- **It is per UTF-16 code unit, and the trait is per `char`.** Mechanical: walk the string's
  `char`s, track how many units each takes, read the entry for that character's *last* unit. A
  character outside the basic multilingual plane comes out as one advance, correctly — which is
  more than `ui_tui`'s cell counting manages and more than `ui_win32/` in the sibling repository,
  where it is a documented gap.
- **It does not shape.** A combining mark gets an advance of its own instead of folding onto its
  base, so the core believes the text is one mark wider than it looks.

The second one has a consequence that decides the whole shape of this: **if drawing uses the
same advance array**, via `ExtTextOutW` with explicit advances, the caret and the glyph still
agree — the mark is drawn floating in a box of its own, which *looks* wrong but is not
*inconsistent*, and every caret operation stays right. If drawing instead lets GDI place glyphs
freely, the two disagree and the caret is wrong.

Which rules out every half-measure. Measure with DirectWrite and draw with plain `ExtTextOutW`
and DirectWrite calls the mark zero-width while GDI draws it in a box — the same disagreement
from the other side. **The measuring engine and the drawing engine have to be the same one**, so
the real choice is only ever between two whole stacks:

| | Measurement | Drawing | Cost |
|---|---|---|---|
| **GDI, both halves** | `GetTextExtentExPointW` | `ExtTextOutW` + the measured advance array | one file — **chosen for W0–W8** |
| DirectWrite, both halves | `GetClusterMetrics` / `HitTestTextPosition` | `IDWriteBitmapRenderTarget::DrawGlyphRun` behind an `IDWriteTextRenderer` **implemented in Rust as a COM interface** | a factory, a text format per `TextStyle`, that renderer, and a second drawing path — the named upgrade |
| Uniscribe `ScriptStringAnalyse` | full shaping, no COM | its own idiom | declined — deprecated in favour of DirectWrite, at close to DirectWrite's complexity |

The COM callback interface in row two is the real price, and it is the reason not to start there
when the trait is a swap point that costs one file and no core change. That is the property
Path C was chosen for; this shell is its fourth `Metrics` implementation and its second proof.

#### What GDI actually costs, and what is *not* pre-approved

Ranked by how likely anybody is to meet it:

1. Precomposed Latin, Cyrillic and Greek — **nothing**. Nearly every real document.
2. **Decomposed (NFD) text** — the realistic hazard, since macOS hands out NFD and a document
   can carry it.
3. Ligature fonts — `ExtTextOutW` with explicit advances suppresses ligatures anyway. Consistent,
   just plainer than a shaped engine would draw it.
4. Emoji ZWJ sequences — a family emoji becomes several boxes.
5. **Devanagari, Thai, Khmer** — genuinely broken.

Row 5 is **not covered by an existing decision, and an earlier draft of this document wrongly
said it was.** `doc/text-layout.md` excludes **RTL**, which covers Arabic and Hebrew. Devanagari,
Thai and Khmer are left-to-right and complex-shaping, so they fall outside that exclusion and
inside this shell's gap. Naming them here is the whole of their coverage; if that is not
acceptable, the answer is row two of the table above, not a footnote.

**The trigger for the upgrade, written down now so it is not a judgement call later:** the first
time a corpus document, a loop C comparison or a bug report shows a caret landing in the wrong
place because characters were measured as separate boxes and belong in one cluster, `metrics.rs`
moves to DirectWrite — measurement *and* drawing together. Nothing above it changes.

Drawing is therefore `ExtTextOutW` **with the advance array the same measurement produced**, and
that is load-bearing rather than tidy. The grid pane does the same for the same reason, and there
it also forces the fixed character cell the sheet's overflow arithmetic assumes.

A `Metrics` implementation needs a device context and a font. It holds a memory DC
(`CreateCompatibleDC(None)` — no window required, which is what makes decision 5 possible), the
window's current DPI, and a `RefCell<HashMap<TextStyle, HFONT>>` — interior mutability because
the trait's methods take `&self`, and a cache because creating a font per call would be visible
on every keystroke. Every entry is deleted and the map cleared on `WM_DPICHANGED`.

`grind_text::Faces` — which metrics *this* block is set in — sits on top: it answers with the
same provider and a per-kind font, so Down-arrow out of a heading measures the paragraph below
it in the paragraph's font. That bug was found and fixed in the core while `grind-text-gtk` was
being built; this shell inherits the fix and must not re-introduce it by reaching for `Uniform`.

### 4. The menu bar is the surface that grows — there is no command palette

`doc/sheet-shell.md`'s "Four surfaces" gives verbs to a Ctrl+K palette, and the reason is stated
there: a GNOME window has no menu bar, so verbs had nowhere to go but a tool row that grew
without a membership rule. **Windows has a menu bar, and it is the platform's own growable
surface.** The system draws it, so it scales with DPI, follows the theme, and gains Alt-key
navigation and mnemonics with no code here. So the four surfaces map across as:

| `doc/sheet-shell.md` | here |
|---|---|
| Header bar, fixed at five slots | the title bar, and nothing of ours in it |
| Format bar — "reads and writes a property of the selection" | a drawn format strip under the menu, same admission test |
| Context menus | context menus, on the cells, the headers and the text |
| Ctrl+K palette — the one allowed to grow | **the menu bar** |

One surface has been added since, in W2, and it is not a fourth kind: a **strip along the top of
the window holding the name box**, which W3 gave its second half — the formula bar. It is not a
place verbs may go: it holds exactly the two read-outs that name *where the selection is* and
*what is in the cell*, which is why it is a strip rather than a bar. W3 added one more surface
that is also not a fourth kind — a **notice bar** under the strip, for a *state the document is
in* and never for an event that has happened, which is `ui_sheet_gtk`'s line between its banner
and its toasts. Its height is zero when there is no notice, so every rectangle below it is one
arithmetic expression whether or not it is showing, and `notice.rs` holds every sentence it can
say as a portable function with a test. The go-to answer of the
other shells (Ctrl+K's palette in `grind-web`, a popover in `grind-text-gtk`) is this box and
F5, so this shell needs no go-to dialog either.

The admission test transfers unchanged: a **verb** goes in a menu, a **property of the
selection** goes on the format strip, and `CellStyle` + `numfmt::Format` + `CharStyle` bound the
latter. What must not happen is a *toolbar* in the Common Controls v6 sense — that class needs
the application manifest this binary deliberately does not have. The format strip is therefore
**drawn**, in GDI, as part of the window, exactly as the grid is.

`ui_sheet_gtk/src/main.rs`'s `chrome_tests` walk every `gio::Menu` in the window to check that
each item routes somewhere real. The equivalent here is cheaper and better: the menus are
**tables of data** in a portable `menu.rs`, so the test that every command id has a handler and
every handler has an item runs on Linux with no window at all.

**Built in W3**, and the two halves of that check turned out to be different in kind, which is
better than it sounds. That every command is in exactly one menu is a *test*, in `menu.rs`, on
Linux. That every command has a *handler* is the **compiler's**: `win.rs`'s dispatcher matches
on `Command` exhaustively, so a verb added to the table and nowhere else fails the build. A
command's `WM_COMMAND` id is its position in `Command::ALL` plus `FIRST_ID`, written down once,
so the classic Win32 bug of two items sharing an id cannot be spelled here; the child controls
take ids below `FIRST_ID`, which is what keeps a control's notification and a menu click apart
in the one message Win32 uses for both.

### 5. `--render-to`, with no window and no display — *built in W2*

The GTK shells draw one frame to a PNG and exit so that a refactor can be proved by a
byte-identical image. The Win32 version of that is *stronger*, and it falls out of the shape the
code wants anyway: painting is a free function taking an `HDC` and a `RECT`, and the window path
and the render path both call it.

`CreateCompatibleDC(None)` + `CreateDIBSection` gives a drawing surface with **no `HWND`, no
compositor and no display**, which means `--render-to` works on a headless `windows-latest`
runner *and* under Wine on a Linux CI runner. The output is a BMP: a 54-byte header in front of
the DIB bits the section already holds, no encoder, no new dependency, and byte-comparable.

Not a user feature, same as the other two shells.

**Built, and it works exactly as described.** `Dib` in `gdi.rs` is `CreateCompatibleDC(None)` +
`CreateDIBSection`, and `win::render` is a second *caller* of `draw_frame` rather than a second
drawing path — the same `opened()` state, the same `paint`, no `HWND` anywhere. Two things are
pinned rather than read, because the output's only purpose is to be compared with another one:
the frame is 1280×800 at 96 dpi, and **the theme is forced to light**, so a screenshot does not
depend on what the machine running it has under `Themes\Personalize`.

The section is **24-bit and bottom-up**, which is what makes the "no encoder" claim true rather
than nearly true: a bottom-up 24-bit DIB's bits *are* a `.bmp` file's pixel data, padding and
all, so writing one is a 54-byte header in front of them. A 32-bit section would have carried a
fourth byte per pixel that GDI leaves undefined — precisely the thing a byte-for-byte comparison
must not depend on.

### 6. The clipboard is the system's, and this shell is ahead of two others

`CF_UNICODETEXT` with tab-separated text — the shape every other spreadsheet reads — plus
`clear_range` for cut and `enter_range` under the paste. This is the one place where the
Windows shell is *ahead* rather than behind: `grind-tui` has only its own vi register (a
terminal cannot reach a system clipboard without a protocol the host may not speak) and
`grind-text-gtk` has no clipboard at all. Interop with LibreOffice Calc and Excel is W4's exit
criterion, both directions.

### 7. Every modal dialog follows one shape, and it is not obvious

`MessageBoxW`, `IFileDialog` and any future find bar **run their own message loop**. While one
is up this window still receives `WM_PAINT`, the window procedure is re-entered, and it produces
a *second* `&mut App` while the first is alive. That is aliasing UB even though it appears to
work, and reading the code does not reveal it — it was found in the sibling repository by
driving the shell under Wine.

The rule, therefore, and it is normative: **a handler that opens a modal borrows the `App`
briefly on each side of the dialog and never across it.** `on_close` in
`editor/ui_win32/src/app.rs` is the worked example — one borrow to compose the question, no
borrow while the dialog runs, one borrow to act on the answer.

**W3 gave the rule teeth**, and made it a rule about a *file* rather than about a habit: every
modal this shell opens lives in `dialog.rs`, with the warning in its module comment, so a
caller can see what it is calling. `offer_to_save` is the worked example here — one borrow for
the flag and the name, no borrow for `confirm_close`, one borrow to act. The same shape covers
`sheet_add`, `sheet_rename` and `sheet_delete`, each of which reads what the prompt needs,
prompts, and then takes a *fresh* borrow to apply the answer.

The rule has a second half W3 discovered rather than inherited, and it is about the **observer**
rather than about dialogs. `App::mutate` notifies with its write lock dropped but still inside
the call that made the change — so an observer that *sends* a message would re-enter the window
procedure while the handler that called `enter` is holding `&mut State`. `Changed` therefore
**posts**, and the dirty flag is set when the message is handled rather than when it is raised.
That is also why this shell needs no "a load is not an edit" flag, where `ui_sheet_gtk` does:
the observer is registered on the `App` after the file has been read, so during a load there is
nobody to tell.

There is one more thing `dialog.rs` holds and the plan did not anticipate: a **text prompt**,
for naming a sheet. `MessageBoxW` cannot ask for a string and this binary has no resources, so
`DialogBoxIndirect` would mean building a `DLGTEMPLATE` by hand — more code than a window and
less readable. It is a popup of this shell's own with `IsDialogMessageW` supplying what a dialog
manager would have: Tab between the controls, Enter for the default button, Escape for Cancel.

The file dialogs are `IFileDialog` (COM, Vista+) rather than `GetOpenFileNameW`, because
`IFileDialog` is the modern dialog and needs no manifest to be one, and because the `windows`
crate makes COM ergonomic. Its filters follow `doc/flat-first.md`: **the flat form is the
default** — `.fods` / `.fodt` first, then the package, then `.grind` — because in doubt this
project writes the form that diffs.

W3 built both, and running them added one qualification the plan had wrong: that ordering is
**Save's**. The *Open* dialog leads with "All spreadsheets", because a filter there is a way of
finding a file rather than a statement about form, and a user whose documents are `.ods` should
not have to change a drop-down to discover they exist. Nothing in either dialog decides what a
form *is*: `grind_sheet::write_file` reads the extension through `Form::from_path`, which is the
one place in the workspace where an extension decides anything.

## The crate

A `*` marks what W0 through W3 have built; everything else is the plan.

```
ui_win32/
  Cargo.toml            * grind-win32; the `windows` dependency target-gated on cfg(windows)
  src/
    main.rs           *   argv, kind sniff, a message box for errors before a window exists
    args.rs           *   the command line as a pure function (W0)
    win.rs           [W]* the class, the wndproc, the message loop, GWLP_USERDATA, the two
                          child EDITs (name box and editor), the menu bar, the verbs, and the
                          windowless render path
    gdi.rs           [W]* RAII wrappers for fonts and brushes; the back buffer; and the DIB
                          target behind --render-to, with its BMP writer (W2)
    metrics.rs       [W]  Metrics + Faces over a memory DC, with the font cache
    theme.rs         [~]* the palette (portable, incl. the selection wash's `Rgb::blend`) and
                          the registry read + dark title bar [W]
    menu.rs           *   the menus as data, the accelerators, and the command-id table
    notice.rs         *   every sentence the notice bar says, as a pure function
    dialog.rs        [W]* every modal: the file dialogs, the questions, and the text prompt
    sheet/
      geom.rs           * pixels <-> cells, prefix sums over the document's own widths, the
                          strip, and which visible track a cursor may stop on
      keymap.rs         * virtual-key codes -> a motion; the selection; the Ctrl+arrow rule
      status.rs         * the name box, the formula bar, and the status bar's aggregates, all
                          over a real `App`
      state.rs          * Ready / Enter / Edit, the editing state machine, and the two
                          conversions an edit needs (display syntax, UTF-8 bytes -> UTF-16)
      draw.rs        [~]* a viewport painted onto an HDC — and, portable beside it, what a cell
                          *looks like*: alignment, weight and colour resolved from `CellStyle`,
                          and what the selection wash does to it
    text/
      keymap.rs           virtual-key codes -> caret operations
      draw.rs        [W]  laid-out blocks painted onto an HDC, run by run
    code.rs          [~]  the projection pane (D9)
    problems.rs      [~]  the lint pane (D6)
```

`sheet/draw.rs` came out `[~]` rather than `[W]`, and that was worth the split: "a number is
right-aligned unless the document says otherwise" is a rule about *documents*, not about GDI, so
`Appearance::of` resolves a `CellStyle` into an alignment, a weight and two colours in portable
code with tests, and only putting pixels down is behind `cfg(windows)`. The `[W]` half is then
short enough to read in one go, which is the property that matters for a file nobody can run
here.

`[W]` needs Windows; everything else compiles and runs its tests on any host. **That split is
the design, not an accident of it**: it is what lets the Windows shell be developed on the Linux
machine this repository lives on, and `editor`'s `ui_win32/` got 35% of its lines onto the
portable side. The target here is higher, because `geom.rs` and `state.rs` are the two files
with real arithmetic in them and neither needs a window.

`Cargo.toml` gates the dependency the way the sibling repository does:

```toml
[target.'cfg(windows)'.dependencies]
windows = { workspace = true }
```

Only the namespaces used get features. The first `cargo tree` after W0 records what that costs
in lock entries and in a cold build; `windows-sys` is already in `Cargo.lock` transitively, so
part of the machinery is paid for.

## Milestones

Ordered on the same insight `doc/sheet-shell.md` used: **the read-only grid is the highest-risk
item and needs no new core API**, so it goes first and de-risks the custom drawing before
anything depends on it. Every milestone lands green — `cargo test`, clippy clean, `reuse lint`,
`cargo fmt --check`, and the Windows type-check from Linux.

| # | Milestone | Contents | Exit criterion |
|---|---|---|---|
| **W0** | **Plan and wiring** — *done* | this document; `ui_win32/` with `args.rs` (the command line as a pure function, 16 tests) and a `main.rs` that resolves what it *would* open; workspace member; `.cargo/config.toml` with `+crt-static` **and the 8 MB stack reserve**; `.github/workflows/win32.yml`; `-p grind-win32` in `ci.yml`'s build/test/clippy/docs lists; REUSE headers | **Met.** Type-check, clippy and 16 tests green on Linux for the msvc target; the shell links under `cargo-xwin` and imports only OS DLLs; the `windows` cost measured (10 lock entries, 5.0s cold check). The stack overflow that predates this shell is diagnosed and fixed, and the whole core is now known to work on Windows |
| **W1** | **The window, and the read-only grid** — *done* | class + wndproc + `GWLP_USERDATA`; per-monitor DPI v2 before any window exists; theme and the dark title bar; double-buffered paint answering `WM_ERASEBKGND`; the status bar; `sheet/geom.rs` with prefix sums over the document's own column widths through `length_mm`; `sheet/draw.rs` over `get_viewport`; headers; `WM_VSCROLL`/`WM_HSCROLL` and a wheel that honours `SPI_GETWHEELSCROLLLINES` | **Met.** All fifteen R7 and sample documents open under Wine, and a package (`.ods`) as well as a flat file; the wheel and both scrollbars move the view; columns are as wide as the document says and hidden tracks are gone; 42 tests on Linux, including the cell-rect ⇄ hit round trip. Two bugs found by *running* it and fixed — see below |
| **W2** | **Selection, navigation, and an assertable frame** — *done* | `sheet/keymap.rs` (portable, with the VK constants pinned against `winuser.h` by a `cfg(windows)` test); arrows, Ctrl+arrows, Home/End, PageUp/Down; click and drag; header selection; `sheet/status.rs`'s aggregates; the name box; and decision 5's DIB render target | **Met.** The sample document is navigable by keyboard alone — arrows, Ctrl+arrows, Ctrl+Home/End, PageUp/Down, Ctrl+A, and F5 into the name box — with the view following the cursor; `--render-to` is byte-identical across two runs under Wine, and `win32.yml`'s new `render` job asserts the same on Windows. 79 tests on Linux. One bug found by *running* it — see below |
| **W3** | **Sheet editing** — *done* | the child `EDIT` serving as both formula bar and in-cell editor; Enter/Esc/F2/typing-replaces through `state.rs`; `App::enter`; Delete → `clear_range`; undo/redo; recalculation and the notice bar; open/save/save-as through `IFileDialog` with the three forms; the three-button close confirmation in decision 7's shape; the `*` dirty marker in the title; sheet add/rename/delete; and the menu bar decision 4 makes this platform's growable surface | **Met.** Typing, F2, a double-click and a click on the formula bar all open the editor; a formula is typed in display syntax and stored in ODF's; one that will not parse keeps the edit open with the caret on the problem and says so in the notice bar; Delete, Ctrl+Z, Ctrl+Y and F9 do what they say; the Sheet menu adds, renames — carrying every reference with it, D10 — and deletes; Ctrl+S writes a document that `grind lint` reads back clean. 104 tests on Linux. Two bugs found by *running* it, one of them a crash — see below |
| **W4** | **The clipboard** | `CF_UNICODETEXT` TSV copy, cut and paste over `enter_range` / `clear_range` | copy here → paste into LibreOffice Calc and into Excel, and back, both directions |
| **W5** | **The text pane** | `metrics.rs` per decision 3 and `Faces` over it; `text/draw.rs` drawing `App::layout_block` run by run; a real `CreateCaret` caret; `WM_CHAR` plus the IME path (`WM_IME_*`, `ImmSetCompositionWindow` at the caret); `App::type_markdown`; selection by Shift+arrow, Shift+click and drag; the format strip over `char_style` / `set_char_style`; block kinds, outline and go-to for `p12` / `#intro` / `§2.1.3` | every feature `examples/sample-text.sh` builds is visible and editable, and a test asserts that this pane and `grind text view --width` break the same text at the same places when both are given `Fixed` |
| **W6** | **The three shared panes** | the code view (D9, read-only, over `Projection`'s four line questions, with the line the selection is on marked); the lint pane (D6, every row a jump); the view-mode overlays (V7, `CellRole::marker` and `NameAnchor`, and `:names`' equivalent for the text pane) | all three reachable from whichever pane they apply to, and opening every overlay on every R7 document then saving leaves the bytes identical |
| **W7** | **Chrome and the accessibility floor** | the menus final, as data; context menus on cells, headers and text; the accelerator table; About with `grind_core::build_info`; the key list; `WM_SETTINGCHANGE` following the user's theme; `WM_DPICHANGED` rebuilding fonts and taking the suggested rectangle | the portable menu-table test passes: every command id has a handler and every handler an item. `accesskit_windows` is **named and deferred**, and the system caret is the floor |
| **W8** | **Packaging** | the release artifact off `windows-latest`; the import-table check as a gate; an icon and version resource; the answer to file associations written down | the artifact opens both document types on a clean Windows install with no other install of any kind |

**W5 is the milestone to be nervous about**, not W1. The grid is arithmetic this project has
done three times; the text pane is the first time `layout::Metrics` meets a proportional font
with a real shaping engine behind it, and decision 3 is a bet that GDI's non-shaping answer is
good enough for long enough.

## What it will not do

R10 allows per-shell feature gaps and requires them to be named. These are the named ones.

**Not drawn, kept intact.** A **chart** in a file is read, kept and written back untouched, and
nothing here draws one — the same position `grind-tui` takes, and it is a deliberate stop rather
than a stub. An **image** in a text document, likewise.

**Waiting on the core.** Wrapped cells and **row auto-height** need L3 (`ui_sheet_gtk`'s
measurement moving onto `layout::Metrics`); until it lands, a document's *stored* row heights
are honoured and no row is measured from its contents. **Pagination**, **printing** and **RTL**
are gated in `doc/not-doing.md` and `doc/text-layout.md` and are not this shell's to open.

**Deferred by decision, reachable from the CLI (R9).** A hidden row or column is drawn as
**gone**, with none of `ui_sheet_gtk`'s marker straddling the boundary — so this shell shows what
the document says and offers no way to *unhide* from the grid. `grind sheet hide --unhide` does
it, which is the R9 answer. W2 did not add the marker: the cursor now steps *over* a hidden
track (`keymap::onto_visible`, above), which was the urgent half, and a hit-test target that is
one pixel wide is a chrome question rather than a navigation one. No **autoscroll on a drag**
past the edge of the window — a drag stops at the last visible cell, and Shift+arrow, which does
scroll, is the way to select further. No **zoom** before W8. No **point mode,
autocomplete or signature hints** while typing a formula — `doc/sheet-shell.md`'s M6, the single
largest piece of the GTK window, and nothing about it is Windows-shaped. No **filter UI**: a
filter in a file folds its rows away and nothing here creates one. No **find/replace over
cells**. No **conditional formatting UI**, which exists in no shell. No **command palette**, by
decision 4.

W3 adds three of its own, each smaller than it sounds. There is no **recent-files list**, where
`ui_sheet_gtk` has `gtk::RecentManager`: Windows' equivalent is `SHAddToRecentDocs` plus a
registered ProgID, so it belongs with W8's file associations rather than in front of them. There
is no **greying of unavailable verbs** — Undo is enabled with nothing to undo, and pressing it
does nothing rather than something wrong — because `MF_GRAYED` means tracking menu state on
every change, and W7 is where the menus are finished. And the **sheet is chosen from a menu
rather than from a tab strip**: Ctrl+PageUp/PageDown and Sheet ▸ Next/Previous reach every sheet,
the status bar says which one and how many, and `doc/sheet-shell.md` removed its own tab strip
for looking like a ribbon.

**System-drawn, and therefore not themed by us.** The message boxes and the sheet-name prompt
are drawn by Windows in the *system* colours, so in dark mode they are the light dialogs Windows
itself still draws for a `MessageBoxW`. The window and its two child `EDIT`s do follow the theme
— the latter through `WM_CTLCOLOREDIT`, which is the only lever a control that paints itself
offers. Making a message box dark means not using a message box, which is a worse trade.

**The toolkit's own limits.** No **Mica**: `DWMWA_SYSTEMBACKDROP_TYPE` is reachable and shipped,
but a backdrop only shows through pixels the application does not paint, and a GDI window that
fills its client area with an opaque brush paints all of them. *This is reasoned rather than
measured*, and it is flagged as such in the sibling repository too. The dark title bar
(`DWMWA_USE_IMMERSIVE_DARK_MODE`) is the part that survives and is implemented. No **Fluent
controls** and no v6 **toolbar**, both downstream of having no manifest. No **shaping** at all
while `metrics.rs` is GDI's: decomposed text, ligatures, emoji sequences and the LTR complex
scripts (Devanagari, Thai, Khmer) are each drawn as separate boxes. Decision 3 ranks these by
likelihood and names the trigger; the LTR complex scripts in particular are a gap of this
shell's own making, **not** something `doc/text-layout.md`'s RTL exclusion already covered.

**Accessibility.** A painted GDI window exposes no UI Automation tree. The floor is the
**system caret** — a real `CreateCaret` caret rather than a painted rectangle, so Windows
reports its position to assistive technology and to IMEs for free. `accesskit_windows` is the
fix if this ever matters, and naming it is not the same as planning it.

## Verification

**On this Linux machine, and in CI on an Ubuntu runner:**

```sh
rustup target add x86_64-pc-windows-msvc          # already installed here
cargo check   -p grind-win32 --target x86_64-pc-windows-msvc
cargo clippy  -p grind-win32 --target x86_64-pc-windows-msvc --all-targets -- -D warnings
cargo test    -p grind-win32                      # the portable half: geom, keymap, state,
                                                  # status, menus and notices — 104 of them
```

`cargo check` never links, so it needs no MSVC and no Windows. This catches a `windows`-rs API
change in about a minute rather than on the Windows runner, and it is the capability a C# shell
could not have in any form.

**Driving it on Linux — an inspection aid, never a build path.** `scripts/run.sh` has a `win32`
arm, so the whole of it is one command on the sample document every other shell uses:

```sh
sudo dnf install clang lld wine xorg-x11-server-Xvfb ImageMagick
cargo install cargo-xwin

scripts/run.sh win32                # builds the sample, links with cargo-xwin, runs under Wine
scripts/run.sh win32 book.fods      # or a document of your own
```

It builds **debug**, which works only because `.cargo/config.toml` reserves 8 MB of stack; with
the MSVC default of 1 MB it overflowed before `main` did anything (W0, below). Under a nested
display it is screenshottable and drivable, which is what caught W1's two drawing bugs:

```sh
Xvfb :99 -screen 0 1400x900x24 &
DISPLAY=:99 scripts/run.sh win32 &
DISPLAY=:99 import -window root /tmp/shot.png     # python-xlib + XTEST drives the mouse
```

From W2 there is a second, quieter way in, and it needs no display at all — which makes it the
faster loop for anything about *drawing* rather than about input:

```sh
env -u DISPLAY wine target/x86_64-pc-windows-msvc/debug/grind-win32.exe book.fods \
    --render-to /tmp/frame.bmp
```

`cargo-xwin` links a real msvc binary with `lld-link` against Microsoft's own CRT and SDK. **The
artifact that ships comes off `windows-latest` and nowhere else**, the same rule
`.github/workflows/gtk.yml` states for the GTK shells: "its own workflow on its own runner,
never cross-compiled". A green Wine run is not evidence that Windows is happy, and putting one
in CI would quietly turn a debugging convenience into a release path.

It earns its place because of what it catches. In the sibling repository, driving the window
under Wine through XTEST found decision 7's aliasing bug, which reading the code did not; here it
found all three of W1's, W2's one, and **both of W3's — of which one was a hard crash that
`cargo check`, clippy and 104 unit tests all passed clean.** None of them survived a single
screenshot.

On Windows there is nothing to arrange and the script has no arm for it: `cargo run -p
grind-win32 -- book.fods` is the whole of it.

**What Wine cannot speak for**, and what therefore needs a real Windows machine before any claim
about it is made here:

- `DwmSetWindowAttribute` is largely inert, so the dark title bar is unverified.
- The theme registry key does not exist in a fresh prefix, so the shell takes its light-mode
  fallback. Writing it by hand (`wine reg add …\\Themes\\Personalize /v AppsUseLightTheme /t
  REG_DWORD /d 0`) does exercise the dark palette here, and W3 used it to check the one thing
  this shell does *not* paint itself: the child `EDIT`s answer `WM_CTLCOLOREDIT` and come back
  dark with light text, where a control left alone is a white field in a dark window.
- Consolas is absent, so every screenshot exercises the `FIXED_PITCH | FF_MODERN` substitution
  path rather than the intended font.
- The IME path, and clipboard interop with real Excel. `IFileDialog`'s COM path *does* run
  under Wine — W3 opened a document through it — but it is Wine's own dialog, and how it looks
  and behaves on Windows 11 is unverified.
- The `*` dirty marker in the title, which Wine under a bare Xvfb draws no caption for.
- Per-monitor DPI, `WM_DPICHANGED`, and how any of it looks under Windows 11's compositor.

**In CI (`win32.yml`), three jobs:**

1. `check-from-linux` (ubuntu) — `cargo check --target`, `cargo clippy --target`, and
   `cargo test -p grind-win32` for the portable half.
2. `build` (windows-latest) — test, clippy, release build, then **read the import table back**
   and fail on `vcruntime140*.dll`, `msvcp140.dll`, `ucrtbase.dll`, `api-ms-win-crt-*`,
   `mscoree.dll`, `hostfxr.dll`, `microsoft.windowsappruntime*` or `microsoft.ui.xaml*`. The
   binary is never *run* there: it is a GUI-subsystem application whose argument errors go into
   a message box, and a message box on a headless runner waits forever.
3. `render` (windows-latest) — **built in W2.** `--render-to` on a fixture, twice, compared byte
   for byte, with the size and the `BM` magic asserted so that two *empty* frames cannot agree
   their way past it. This is the job that makes a drawing refactor provable. It is the one
   place the binary *is* run on a runner, which is safe because this path opens no window and no
   message box on success — and the job carries a timeout, because on failure it would. The
   frame is uploaded as an artifact, which is how the committed fixture the plan asked for gets
   produced: a `.bmp` from Wine cannot stand in for one from Windows, since every glyph there
   comes from a substituted font.

## Risks

- ~~**The `windows` crate's build cost.**~~ **Measured in W0 and it is not a risk.** With eight
  namespace features rather than the default, it adds **10 crates** to `Cargo.lock` (293 → 304,
  the eleventh entry being `grind-win32` itself) and a **5.0s cold `cargo check`** for the whole
  shell, 3.7s incremental. The `windows-sys` machinery was already in the lock file
  transitively. The fallback below is therefore not needed and is recorded only so the reasoning
  survives.
- **The `windows` crate's build cost, as it was feared.** It is enormous generated code gated by
  feature. Only the
  namespaces used get enabled, and W0's exit criterion includes measuring what it actually costs
  in lock entries and cold-build seconds. If it is bad, `windows-sys` (raw FFI, no COM) is the
  fallback and the price is `IFileDialog` — decision 7 would revert to `GetOpenFileNameW` and
  the old-style dialog.
- **`unsafe`, for the first time in this workspace.** There are exactly three `unsafe` blocks in
  the repository today and all three are `std::env::set_var` in one test in
  `core/src/locale.rs`. A window procedure storing a `Box` pointer in `GWLP_USERDATA` is the
  standard Win32 arrangement and it is still unsafe. Mitigations: it is confined to the `[W]`
  files, decision 7 is a written rule rather than a habit, and every GDI object gets an RAII
  wrapper in `gdi.rs` because a leaked `HFONT` per keystroke is the classic version of this bug.
- **Two panes double the surface.** Mitigation is `ui_tui`'s shape exactly: a `Shell` trait with
  three methods for the *message loop*, and no attempt at an abstraction over documents. The
  core's own `core/src/observer.rs` records why there is no `Editor` trait yet; this shell is
  the third caller and may be what finally answers it — but it does not get to guess.
- **Scope creep toward Fluent.** The answer is in `doc/decision-win32-shell.md`'s "When to
  revisit": if `windows-reactor` reaches 1.0 *and* the Windows App SDK becomes part of Windows,
  reconsider on the spot. Not before.
- **Nobody may ever run it on a clean Windows install**, in which case the central claim is
  untested and the import-table check is doing all the work. This is the honest failure mode and
  it is the same one the sibling repository has.

## Evidence

Measured on 2026-09-04, on the development machine, before any of the above was written:

| Claim | How it was checked |
|---|---|
| `grind-core`, `grind-sheet` and `grind-text` type-check for `x86_64-pc-windows-msvc` on Linux | `cargo check -p grind-core -p grind-sheet -p grind-text --target x86_64-pc-windows-msvc` — clean, 14.58s |
| The whole CLI **links** for Windows on Linux, statically | `RUSTFLAGS="-C target-feature=+crt-static" cargo xwin build -p grind-cli --target x86_64-pc-windows-msvc` — exit 0; the link pulled `libcmt.lib` and `libvcruntime.lib`, the static CRT |
| A statically linked `grind.exe` imports only DLLs Windows ships | `objdump -p`: `KERNEL32.dll`, `ntdll.dll`, `bcryptprimitives.dll`, `api-ms-win-core-synch-l1-2-0.dll` — no `vcruntime140.dll`, no `ucrtbase.dll`, no `api-ms-win-crt-*` |
| The toolchain this plan assumes is already present here | `rustup target list --installed` lists `x86_64-pc-windows-msvc`; `cargo-xwin`, `wine`, `lld-link` and `clang` all on `PATH`; the xwin CRT+SDK cache is populated |
| The workspace has almost no `unsafe` to lose | `grep -rn unsafe --include='*.rs'`: three blocks, all `std::env::set_var` in `core/src/locale.rs`'s tests |
| `windows-sys` is already in `Cargo.lock` transitively | `grep windows Cargo.lock` |

Added in W0, once the crate existed:

| Claim | How it was checked |
|---|---|
| A debug Windows build overflowed its stack, and the reserve was the cause | `objdump -p`: `SizeOfStackReserve 0000000000100000` before, `0000000000800000` after `/STACK:8388608`; the crash is gone |
| The formula engine, reader, writer and linter all work on Windows | `wine grind.exe sheet set/recalc/view/lint` over a `.fods` built from nothing — `=SUM([.A1:.A2])` returned 9 |
| `grind-win32` type-checks **and lints** for Windows on Linux | `cargo clippy -p grind-win32 --target x86_64-pc-windows-msvc --all-targets -- -D warnings`, clean |
| Its portable half runs its tests on Linux | `cargo test -p grind-win32`: 16 passed — argument handling, the type-flag reconciliation, and that the *bytes* decide the document type rather than the extension |
| The shell links for Windows and imports only OS DLLs | `cargo xwin build`, then `objdump -p`: `user32`, `KERNEL32`, `ntdll`, `bcryptprimitives`, one api-set |
| The `windows` crate is cheap when its namespaces are gated | 10 crates added to `Cargo.lock` (293 → 304 including `grind-win32`); `cargo check` 5.04s cold, 3.68s incremental |
| `cargo build --target` does **not** work on the Linux runner and `cargo check` does | exit 101 at the link step; `win32.yml` says so in a comment so nobody re-adds it |

Added in W1, once there was a window:

| Claim | How it was checked |
|---|---|
| Every R7 document and every sample opens, and so does a **package** | fifteen `.fods` under Wine, one at a time, each reporting a real top-level window rather than a message box; then `grind convert` to `.ods` and the same again — the zip reader path |
| A document's own column widths decide the geometry | `examples/sample-sheet.sh`'s budget, whose column A is set wide, drawn wide; and `Sizes::from_lengths` asserted against `2.5cm` and `10mm` at two DPIs |
| A hidden row or column is *gone* | `hidden-rows-cols.fods`: the header band reads A B D and the row band 1 2 4 |
| The view scrolls, by all three means | XTEST under Xvfb: six wheel notches moved the top row from 1 to 19 (three lines a notch, the default), the horizontal arrow moved column A to D, and the vertical arrow stepped a row at a time |
| Nothing flickers, and the window paints its own background | every frame goes onto a `CreateCompatibleDC` back buffer and is blitted once; `WM_ERASEBKGND` answers 1 so the class brush is never painted, and there is no class brush to paint |
| The whole thing still lints and tests on Linux | `cargo clippy` clean for **both** targets, `cargo test -p grind-win32`: 42 passed |
| The shell still imports only OS DLLs, with a window in it | `objdump -p`: `user32`, `gdi32`, `dwmapi`, `KERNEL32`, `advapi32`, `oleaut32`, `ntdll`, `bcryptprimitives`, one api-set — every one of them part of Windows |

Added in W2, once there was a selection:

| Claim | How it was checked |
|---|---|
| The sample document is navigable by keyboard alone | Under Xvfb, driven by XTEST: arrows and Shift+arrows build a rectangle (`C4:E9`, with the status bar reporting `Sum 4670.24834350305 · Count 14 · Average 333.589167393075`); Ctrl+End reaches `J20` and Ctrl+Home `A1`; Ctrl+Down runs to `A19`; Ctrl+A selects `A1:J20`; two PageUps from row 200 land on row 148 with the view moved to match |
| Click, drag and the header bands select what they look like | A press at B10 dragged to E13 gives `B10:E13`; a click on the C header gives `C1:C1048576` and one on the row-4 header `A4:XFD4`; the corner gives `A1:J20`. Every one of them highlights its own header buttons |
| The chrome is not the grid | A click in the status bar and a click on an empty part of the strip both leave the selection exactly where it was — `Hit::Chrome`, which exists so that the corner button's select-all cannot be triggered by the status bar |
| The name box goes where it is told, and cancels | F5 shows the control with the current address selected; typing `g20` and pressing Enter moves to G20 and the box goes back to being drawn; typing `zz` and pressing Escape leaves the selection where it was and says nothing |
| The view follows the cursor | `A200` typed into the name box scrolls the sheet so row 200 is the last one drawn — `Sizes::start_showing`, which walks back a screenful rather than stepping a hundred and seventy-four times |
| `--render-to` draws a frame with no window and no display | Under Wine with `DISPLAY` **unset**: a 3 072 054-byte `.bmp` — 54 bytes of header and 1280 × 800 × 3 of pixels — with the grid, the headers, the name box and the status bar all in it |
| The same frame twice is the same bytes | `cmp` on two consecutive renders: identical. `win32.yml`'s `render` job asserts the same on `windows-latest` |
| The whole thing still lints and tests on Linux | `cargo clippy` clean for **both** targets, `cargo test -p grind-win32`: 79 passed |

Added in W3, once cells could be typed into:

| Claim | How it was checked |
|---|---|
| A value typed lands in the document, formatted, with the cursor moved on | Under Xvfb, driven by XTEST: `12` and Enter into E14 draws `12` right-aligned in that cell — a number, not the control's own left-aligned text — and leaves the cursor on E15 |
| A formula is typed in **display syntax** and stored in ODF's | `=SUM(B3:B4)` typed into E16 shows `720`, and `grind sheet view` on the saved file agrees. The conversion is `state::to_store`, which is `formula::display::from_display` and nothing else |
| A formula that will not parse does **not** commit | `=SUM(` and Enter leaves the editor open with the caret on the problem and the notice bar reading *"Not a formula: expected a value. Esc leaves the cell as it was."*; Escape then leaves the cell exactly as it was |
| Escape throws an edit away and Enter does not | `999` then Escape leaves the cell empty and closes the editor; the same text then Enter stores it |
| F2 and a double-click open the cell rather than replace it | A double-click on B3 opens the editor holding `500` — `App::input_text`, so a formula would come back in display syntax and a date in the ISO spelling that types back in |
| Clicking the formula bar edits the cell it is showing | The control appears **on the bar**, holding `500`, while the cell underneath keeps showing its formatted `500.00 €` |
| The bar mirrors an in-cell edit as it is typed | `EN_CHANGE` repaints the strip and only the strip; the drawn bar reads what the control holds |
| Delete empties a cell, and Ctrl+Z brings it back | Delete on B9 (a `MAX` formula) empties it; Ctrl+Z restores it, and Ctrl+Y removes it again |
| F9 says what it did | *"Every formula already holds what it computes."* on an up-to-date document, and a count when there is one. `notice.rs` holds the sentences and their plurals, tested on Linux |
| The menu bar is Windows' own | Alt+S opens **Sheet** with its mnemonics underlined and `Ctrl+PgDn` / `Ctrl+PgUp` drawn beside the items that have them; Alt+F, x reaches Exit |
| Adding, renaming and deleting a sheet all work, through the prompt and the question | Sheet ▸ Add gives `Sheet3 (3 of 3)`; Sheet ▸ Rename on `Budget` → `Ledger` reports *"4 references rewritten to follow the rename"*; Sheet ▸ Delete asks *"Delete Archive and everything on it?"* with **No** as the default |
| A rename carries the document with it (D10) | The saved file's named expression reads `[$Ledger.$B$2:.$B$7]`, and `grind lint` finds nothing but the two pre-existing chart warnings |
| Ctrl+S writes a document the rest of the suite reads | The file's bytes change, `grind --format json info` reports the renamed sheets, and `grind sheet lint` is clean |
| The close question is the three-button one, and Cancel cancels | File ▸ Exit on a modified document asks *"Save changes to edit.fods before closing?"* with Yes / No / Cancel and Yes as the default; Cancel leaves the window open, Yes saves and then closes |
| Opening a document through `IFileDialog` works | Ctrl+O opens the shell dialog with "All spreadsheets" as the leading filter; a path typed into it loads that document and resets the view to A1 |
| The child controls follow the theme | With `AppsUseLightTheme` set to 0 in the prefix, both `EDIT`s come back dark with light text — `WM_CTLCOLOREDIT`, the only lever a control that paints itself offers |
| `--render-to` is still byte-identical, with the formula bar in it | Two consecutive renders `cmp` equal, under Wine with `DISPLAY` unset |
| The shell still imports only OS DLLs, now with COM in it | `objdump -p`: `user32`, `gdi32`, `dwmapi`, `ole32`, `combase`, `oleaut32`, `KERNEL32`, `advapi32`, `ntdll`, `bcryptprimitives`, one api-set. `IFileDialog` costs `ole32` and `combase`, both part of Windows; `shell32` does not appear at all, because the dialog is reached through `CoCreateInstance` rather than by linking a shell entry point |
| The whole thing still lints and tests on Linux | `cargo clippy` clean for **both** targets, `cargo test -p grind-win32`: 104 passed |

What is still unverified by construction: decision 3's measurement, because it does not exist
yet, and — per *What Wine cannot speak for* above — the dark title bar, the DPI path and the
IME, because Wine cannot answer for any of them. W2 added one more to that list: the `render`
job's output has never been produced on a **real Windows** machine, so the committed-fixture
half of decision 5 is deferred until a `.bmp` from that runner can be committed from it. Two
renders agreeing is what is asserted today. W3 takes decision 7's modal shape *off* it — the
modals exist and were driven — and leaves the `*` dirty marker on it, because Wine under a bare
Xvfb draws no caption to read it from.

### What running it found, which reading it did not

Both of these were invisible in review, compiled clean, and were obvious within a second of
looking at a screenshot. They are the argument for the Wine path in miniature.

1. **Every string was drawn with a box glyph after it.** `gdi::wide` NUL-terminates, which is
   what the `…W` entry points taking a `PCWSTR` want — but `DrawTextW` in the `windows` crate
   takes a *slice* and uses its length as the character count, so the terminator was a
   character to draw. GDI has no reason to treat it specially and drew `.notdef`.
2. **A cell whose text contains a line break drew the same box**, for the neighbouring reason:
   `DT_SINGLELINE` does not *ignore* a control character, it draws one. `sheet/draw.rs`'s
   `one_line` now maps every control character to a space, which is a portable function with a
   test rather than a flag on the call.

A third, found by reading the *document* rather than the screen but only after the screen made
it worth checking: **ODF hides a track with `table:visibility="collapse"`, not with a width of
zero**, so a hidden column still carries an ordinary `style:column-width` and reading only the
widths drew it. `Sizes::from_lengths` now takes the hidden list too and applies it last.
`App::hidden_cols`, `manually_hidden_rows` and `hidden_rows` are three separate questions and
this shell asks all three — the same arrangement `ui_sheet_gtk/src/grid.rs` arrived at.

### W2 found one more, and it was invisible on paper for the same reason

**Arrow keys walked into hidden rows and columns.** Two presses of Down in
`examples/sample-sheet.sh`'s budget — which hides rows 2 and 5–7 and column D — put the cursor on
row 5, and the screen showed no cursor at all while the status bar cheerfully reported one. Every
unit test passed: the *selection* was correct, and `moved` is a function of cell coordinates that
has no business knowing about widths.

The cause is this shell's own W1 decision made sharp. `doc/sheet-shell.md`'s window draws a hidden
track as a marker straddling the boundary; this one draws it as **gone**, which is written down
above as a named gap — and a cursor parked on nothing is that gap's edge. `keymap::onto_visible`
is the fix and it is where the rule belongs: after a motion, the *active cell* moves off any
hidden track in the direction it was travelling, and the **anchor is left alone**, because a
selection's corner may perfectly well sit on a hidden track and moving it would silently change
what a later operation covers. `Sizes::nearest_visible` is the same question `scroll_rows`
already asked in W1 — a hidden track occupies no pixels, so stepping onto one changes a number
and not the screen.

### W3 found two, and the first one was a crash nothing else could have caught

**1. Drawing an empty string was an access violation.** Committing an edit moved the cursor onto
an empty cell, the formula bar had nothing to say, and the process died — inside a system DLL,
reading address `2`, with nothing of ours on the faulting frame. Wine's log said
`err:seh:user_callback_handler ignoring exception c0000005` and no more.

The cause is one line of W1's `draw_text` and it is a Rust fact rather than a Win32 one: an
empty `Vec<u16>` performs **no allocation**, so `as_mut_ptr` hands back a dangling
well-aligned address — for `u16`, the literal value `2` — and `DrawTextW` dereferences the
buffer before it looks at the count. The count was zero and the pointer was garbage, which is
exactly the combination a slice-taking API makes easy to write and impossible to see. `wide`
having a NUL in it is what hid this through W1 and W2: `gdi::wide` is used for the `PCWSTR`
entry points and always has at least the terminator in it, while `draw_text` deliberately does
*not* NUL-terminate — see W1's first bug, which is the same distinction from the other side.
Every string W1 and W2 drew had something in it. The fix is a `return`, and the reason is
written where the `return` is.

Nothing short of running it would have found this. It is not a logic error, so no test in
`geom.rs` or `state.rs` could have caught it; it compiles, it is clippy-clean, and it is one
`if` away from an API used correctly everywhere else in the file.

**2. `App::rename_sheet` returns a count, not an index.** The shell stored the answer in
`State::sheet`, so renaming a sheet whose name occurred in four formulas selected *sheet 4* of a
two-sheet document, and the next paint panicked — which in a window procedure is an abort, not a
backtrace. The core's doc comment says exactly what it returns; the shell had assumed the shape
of `add_sheet` beside it.

The fix is two things rather than one, and the second is the interesting half. The call site now
uses the count for what it is — the notice bar says *"4 references rewritten to follow the
rename. Ctrl+Z takes it back."*, which is D10's whole point said out loud. And `State::relayout`
now **clamps the sheet index**, because it is the one function every path that changes the
document ends in: deleting a sheet, undoing an insertion and opening a smaller document all
leave that index pointing past the end, and a painter handed one has nothing useful to do with
it. A shell should not be able to ask for a sheet that is not there, and now it cannot.

A third thing, found by reading rather than by running, and worth the same line of the ledger:
`dialog::confirm` was `MB_YESNOCANCEL`, which makes **Yes** the default. For "delete this sheet
and everything on it" that is a dialog that deletes things when somebody presses Enter twice. It
is `MB_YESNO | MB_DEFBUTTON2` now — two buttons, No default — while `confirm_close` keeps its
first-button default, because there the safe answer *is* the first one.

### The one thing that did not work — found, diagnosed and fixed in W0

`wine grind.exe --version` **crashed**, before any of this existed:

```
thread 'main' has overflowed its stack
=> 0 __chkstk+0x37 ... in grind
   ...
   8 std::sys::backtrace::__rust_begin_short_backtrace<std::process::ExitCode (*)()>
```

Wine 11.0 (Staging), the **debug** build, before `main` had done anything. The stack probe
`__chkstk` failing that early points at the platform's own default rather than at a runaway
recursion: **a Windows main thread reserves 1 MB of stack, where Linux gives 8 MB**, and the
reserve is baked into the PE header at link time. An unoptimised build's frames are several
times an optimised one's, so this is the classic shape of a Rust program that is fine on Linux
and fine in `--release` on Windows and dies in a debug build on Windows.

**Confirmed from the PE header rather than argued.** `objdump -p` on that binary reported
`SizeOfStackReserve 0000000000100000` — 1 MB exactly, the MSVC default, baked in at link time.
The candidate that it was genuinely deep recursion somewhere (`formula::eval` recurses over the
dependency graph rather than sorting one, and the reader's context stack is recursive too) is
ruled out by the backtrace: the overflow is **four frames into our own code**, so this is frame
*size* in an unoptimised build, not depth.

Fixed by reserving 8 MB — what Linux gives — in `.cargo/config.toml`, beside `+crt-static`.
Verified in three steps, each of which is worth more than the one before it: the header now
reads `0000000000800000`; the binary runs; and the whole core works through it —

```
$ wine grind.exe sheet set book.fods A3 '=SUM([.A1:.A2])'
$ wine grind.exe sheet view book.fods A1:A3
3
6
9
$ wine grind.exe lint book.fods
book.fods: no problems found
```

The formula engine, the reader, the writer and the linter all work on Windows. That is a
much better W0 foundation than "it compiles", and none of it needed a line of shell code.

A reserve is address space rather than committed memory, so this costs a 64-bit process nothing
it will notice. It is set in the config rather than worked around per binary because the
alternative — running the real work on a spawned thread with an explicit stack size — is a
change every entry point has to remember, and forgetting it fails only on Windows and only in
debug. `win32.yml` asserts the header on every build so the flag cannot silently stop reaching
the linker.

**Note what this was a fact about.** `grind.exe` is a `grind-cli` build with no shell code in it
at all: this is about running *this workspace* on Windows, and it would have been found the
first time anybody tried, with or without `ui_win32/`.
