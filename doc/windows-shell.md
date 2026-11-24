<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# The Windows shell

The plan for `ui_win32/` — crate `grind-win32`, binary `grind-win32.exe` — and the decisions
behind it. Normative for that directory the way `doc/tui-shell.md` is for `ui_tui/`,
`doc/web-shell.md` for `ui_web/` and `doc/sheet-shell.md` for the spreadsheet's GTK window.

**Status: built.** Its look is Fluent 2's tokens (decisions 9 to 13). A text document is shown
as its printed pages, set by `grind_print` and blitted from its raster with `AlphaBlend`, with the
caret, the selection and the marks drawn over them from `grind_print::place` (`text/pages.rs`);
GDI text is the chrome's alone (`doc/page-model.md`).

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

**L3 has since landed**: `ui_sheet_gtk`'s row auto-height measurement moved onto
`layout::Metrics`, so one breaker serves both applications, and `metrics.rs`'s `Faces` is proof
that a fourth `Metrics` implementation costs this crate nothing new. This shell's *own* row
auto-height is a separate piece of work L3 only clears the way for — this crate has never
measured a row from its contents at all, honouring only the heights a document *stores* — and
remains a named gap below rather than something L3 itself builds.

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

No runtime, no COM apartment beyond what the file dialog needs, and `+crt-static` in a new
`.cargo/config.toml` so the MSVC C runtime is linked in rather than looked for. The claim in
the one-liner above is then *checkable*, and CI checks it by reading the import table back
rather than trusting it.

This is already measured, before a line of the shell exists — see *Evidence* at the end.
`grind.exe`, built for `x86_64-pc-windows-msvc` from this workspace on this Linux machine,
imports exactly four DLLs, all of them part of Windows.

*Since W14 the executable does carry an application manifest* (`ui_win32/data/grind.manifest`,
decision 12) — a resource in the `.exe`, not a file beside it and not a dependency: it asks for the
Common Controls 6 every Windows since XP ships, per-monitor DPI v2, and the Windows 10/11
compatibility context. What W0 meant by "no manifest" was no side-by-side runtime to install, and
that is still true.

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
| **GDI, both halves** | `GetTextExtentExPointW` | `ExtTextOutW` + the measured advance array | one file — **chosen, and built in W5a** |
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
latter. What must not happen is a *toolbar* in the Common Controls v6 sense — W0 refused it for
needing a manifest, and since W14 has one the refusal stands on the better reason: a `ToolbarWindow32`
draws itself in the system's style and ignores this window's palette, which is exactly the half-themed
look decision 10 was written against. The format strip is therefore **drawn**, in GDI, as part of
the window, exactly as the grid is.

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

So where the GNOME window gives the tape (`doc/calc.md`, an experiment) a header-bar button, this
shell gives it **Data ▸ Scratch Calculator…**. It is the list modal again (`tape.rs`, portable):
one row per line with its value, then *Add a Line…*, *Undo*, *Redo* and *Open as Spreadsheet*;
picking a line edits it in the prompt, and emptying it deletes it. The tape stays with the window,
and goes along into the spreadsheet it opens as. Not built: *Name a number* (`Tape::extract`).

### 5. `--render-to`, with no window and no display

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
the frame is 1280×800 at 96 dpi, and **the palette comes from the command line rather than from
the machine**, so a screenshot does not depend on what the machine running it has under
`Themes\Personalize`.

**W10 made that second pin a choice rather than a constant**, and had to. It was `Mode::Light`,
full stop, which kept the output reproducible and had the side effect that the dark palette could
not be rendered *at all* — the window reads the registry and this path did not, so half of
`theme.rs` was shipped unlooked-at, and the first frame `--dark` ever produced had a bug in it
that had been there since W1 (see below). `--dark` names the palette; nothing reads the registry
here; the output is still a function of the command line alone, which is the property that
mattered. `artifacts.yml` renders both and asserts the dark one is a *different* frame, which
fails if the flag ever stops reaching the painter.

The section is **24-bit and bottom-up**, which is what makes the "no encoder" claim true rather
than nearly true: a bottom-up 24-bit DIB's bits *are* a `.bmp` file's pixel data, padding and
all, so writing one is a 54-byte header in front of them. A 32-bit section would have carried a
fourth byte per pixel that GDI leaves undefined — precisely the thing a byte-for-byte comparison
must not depend on.

### 6. The clipboard is the system's, and this shell is ahead of two others

`CF_UNICODETEXT` with tab- and CRLF-separated text — the shape every other spreadsheet reads —
plus `clear_range` for cut and `enter_range` under the paste. `clipboard.rs` is the only file
that opens the clipboard, on `gdi.rs`'s pattern: one `OpenClipboard`/`CloseClipboard` pair per
call, closed on every path including the early returns, so a half-finished copy cannot leave it
open for the rest of the process. `grind_sheet::clip` is the portable half either side of it —
the codec between a rectangle of `App::input_text` and a `String`, tested on any host. It was
`sheet/clip.rs` here until the macOS shell would have made it a fifth copy, and was hoisted in
that shell's M1. This is the one place where the Windows shell is *ahead* rather than
behind: `grind-tui` has only its own vi register (a terminal cannot reach a system clipboard
without a protocol the host may not speak) and `grind-text-gtk` has no clipboard at all.

What was verified under Wine: the round trip inside this shell itself — copy, cut and paste
over the sample document, through the real `CF_UNICODETEXT` API rather than a shortcut, with
recalculation following a pasted value across the sheet. What was **not**: pasting into a
LibreOffice Calc or Excel window on the same machine. A bare Xvfb has no running clipboard
manager to bridge Win32's clipboard to the X11 `CLIPBOARD` selection — `xclip -o` reports no
owner after a Ctrl+C that Wine's own `GetClipboardData` reads back correctly a moment later —
so cross-application interop joins the IME and real Excel on *What Wine cannot speak for*'s
list below, unchanged from what this section already named before W4 was built.

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
crate makes COM ergonomic. Its filters follow `doc/flat-first.md`, the same rule the two GTK
shells' `*_filters`/`*_save_filters` implement: **the flat form is the default on Save** —
`.fods` / `.fodt` first, then the package, then `.grind` — because in doubt this project writes
the form that diffs, and Save offers only the kind the open pane holds, so a text document
suggests `Untitled.fodt` rather than a spreadsheet's `.fods`. **Open offers one combined filter
and prefers neither form** — a user looking for a document does not know which physical form it
is in, and since one window here can hold either document kind the filter combines both kinds'
extensions too, rather than a leading "All spreadsheets" that hid `.fodt` behind a drop-down.
Nothing in either dialog decides what a form *is*: `grind_sheet::write_file`/
`grind_text::write_file` read the extension through `Form::from_path`, which is the one place in
the workspace where an extension decides anything.

### 8. Help while typing is a **band**, not a popup

`ui_sheet_gtk` floats a `gtk::Popover` under the cell being edited, with eight offers and their
summaries. This shell draws one line under the strip instead, in the same manner and the same
place as the notice bar, and the reason is what a popup costs on Win32 rather than what it
gains: it is a second top-level window with its own class, its own theming (a `LISTBOX` paints
itself, so `WM_CTLCOLORLISTBOX`), its own DPI answer, and a focus problem — the whole point is
that typing carries on into the editor underneath while the list narrows, so the popup must never
take the keyboard. A band this window draws itself has none of those, follows the theme by
construction, sits beside the formula bar the text is mirrored in, and — being drawn rather than
a control — appears in `--render-to`'s windowless frame like every other read-out here.

What it costs is the vertical list: five names on one line rather than eight rows each with its
summary. So the *chosen* offer's summary is drawn after the names, which is the line a reader
actually wants, and a longer list says how much of itself it is not showing (`+12`). One more
keystroke narrows it.

Two bands, not one, and they stack: a formula that will not parse leaves a notice up **while the
edit is still open**, which is exactly when a signature hint is worth most. Sharing one row would
mean the more useful of the two hiding the other.

### 9. One accent — **the user's own**, tinted until it can be seen

W9 refused to read the user's accent colour. The argument was that half the values in that
picker are colours a one-pixel selection edge disappears into: it is chosen for a title bar,
where nothing has to stay legible *through* it, and one accent this shell owns is one it can
guarantee reads against both grounds.

**That is true of the accent as the user picked it, and false of the accent as Windows itself
draws it**, which is what W10 noticed. WinUI never paints `SystemAccentColor` onto a surface. It
paints `SystemAccentColorDark1` on a light one and `SystemAccentColorLight2` on a dark one, and
those are tints of the same hue at a lightness chosen so they read. The objection was therefore
to a step nobody takes.

So `theme::accent_for` takes the same step — one ramp position by mode — and then does the thing
a fixed ramp still cannot promise: it keeps moving the lightness until `Rgb::contrast` against
that palette's own document ground clears 3.0, WCAG's floor for a shape you have to be able to
see. The hue is left alone throughout, so it is still recognisably the colour the user chose.

What makes this *safe* rather than optimistic is that it is a pure function of one colour, and
therefore checkable with no Windows and no user:
`any_accent_at_all_is_legible_once_it_has_been_tinted` sweeps every hue at every lightness at
three saturations, in both palettes, and asserts the contrast floor on the page, the contrast of
the accent against its own wash (which is where a selected header button's letter is drawn), and
the contrast of whatever is written *on* the accent. A sweep rather than the forty-eight swatches
the Settings app offers today, because the swatch list is a fact about one Windows build and this
is a fact about the function.

Three things stay as W9 left them. The **fallback is the suite's own blue** — a machine that has
said nothing, every `--render-to` frame, and every Wine run get `PALETTE`'s `blue`, so the mark
still ties to the icon and the two GTK apps. The **wash** a selected cell moves towards is the
accent as the *user* chose it rather than the tint, because a wash is diluted to a fifth before
it touches a cell and the legibility argument that moves the neat colour does not apply to it.
And `grind_core::search::score` is still unused here, because there is still no palette.

### 10. Fluent by **measurement**, not by toolkit

This window has no Fluent controls and will not get any: they are downstream of the Windows App
SDK decision 2 rules out. What it *can* have is every number Fluent publishes,
and that turns out to be most of what "looks like a Windows 11 application" means.

The distinction matters because the obvious reading of "no Fluent" is "so it will look like
Win32", and W9's chrome is what that reading produces: one grey for every band, one size of type,
every measurement the smallest its content fits in, no state under the pointer. None of that was
forced by GDI. It was the absence of a written-down alternative.

So the alternative is written down, in `theme.rs`, as three tables a drawing call is handed and
cannot reach past:

- **Colour.** Fluent 2's neutral ramp, resolved to **opaque values**. Fluent states most surface
  tokens as a white or black at some alpha over the layer beneath, and `FillRect` has no alpha —
  so the composite is done once, here, and written down as the colour it comes out as. That is
  the move `Rgb::blend` already made for the selection wash, applied to the whole palette.
- **Three grounds, where there was one.** A window is a `backdrop` with chrome on it, a
  `background` where the document is — the grid's paper, the text pane's page — and a `card` for
  the things you type in and click. Fluent calls those the base, a layer and a control fill.
  `the_document_and_the_window_are_different_surfaces` asserts all three stay distinct and that
  the document is the *lighter* of the first two **in both palettes**, which is Fluent's layering
  rather than an inversion of it.
- **Type.** A ramp — Caption 12, Body **14**, and the two the documents own — where there was one
  size. Windows 11's body text is 14 pixels, not the 12 a Win32 window has drawn since 1995, and
  a window that sets its status bar and its formula bar identically reads as undesigned. A cell's
  own text is the one size that did *not* move, and the reason is in the constant's own comment:
  a document's column widths were chosen against a size near it, so raising it is data loss
  dressed as typography.
- **Space.** Fluent's four-pixel grid, its 32-pixel control height, and its two corner radii — 4
  for anything you click or type in, 8 for a surface that holds other things. The strips are
  44 tall because that is one control plus its surround, not because 44 looked right.

Two consequences worth naming. A band that is *one sentence about something* — the notice bar,
the assist band — is drawn as an **inset card** rather than edge to edge, because that is what an
`InfoBar` is; and a **control responds to the pointer**, which needed the one piece of state in
this shell that exists purely so something looks alive (`Text::hover`) and is what most gives a
custom-painted window away when it is missing.

The **modals follow the theme too** (`dialog.rs`), which is where this stops being a repaint: a
white listbox inside a dark window is what "not themed" looks like at its worst, and the chooser
*is* a listbox. W10 got the grounds, the edit box and the list's rows there through
`WM_CTLCOLOR…`, and left four things system-drawn — the push buttons, the list's selection bar,
the menu bar and the message boxes. **W14 drew all four** (decisions 12 and 13).

### 11. A window with no document shows the **choice**, not a spreadsheet

Until W11, `grind-win32` with no file opened an empty spreadsheet, and `--text` was the only way
to say otherwise. Decision 1 is what makes that wrong: **one binary, both document types**. A
binary that holds two applications and picks one of them before being asked is wrong half the
time, and it is wrong in the way that costs most — a person who launched it from a Start menu
tile, with nothing to say which of the two they meant, gets a grid and has to work out that File ▸
Open is not what they wanted either.

So a window with nothing to show shows the *choice*: **New Spreadsheet**, **New Text Document**,
**Open a Document…**, as three cards. `welcome.rs` is the pane, and the three things about it
worth writing down:

- **A pane, not a dialog.** `dialog::choose` was the cheap answer and the wrong one. A modal in
  front of an empty grid is still a window that has already decided, and cancelling it leaves
  that decision behind. `Pane` is this shell's existing shape — one window, one pane, decided by
  what was opened — and `Pane::Welcome` extends it to "nothing was opened" with no new machinery:
  every handler that belongs to one document type already asks `with_sheet`/`with_text` and
  already gets `None` for the other.
- **Every card is a `Command`.** Not a private verb — the card runs the menu item. That is
  decision 4's rule holding at the one surface that could most easily have broken it, and
  `welcome.rs`'s own test asserts each choice is in a menu and that the accelerator printed on the
  card is the one the menu bar carries.
- **The menu bar answers a `Surface`, not a `DocumentKind`.** W5b's `applies_to` asked which
  document type a verb belonged to; with no document open that question has no answer, and
  guessing one is the guess this whole decision removes. `menu::Surface` is `Welcome` or
  `Document(kind)`, and over the welcome pane the bar is **File and Help** — Save, Undo, Go To
  and the rest are omitted by the same rule that already omits `&Sheet` over a text document.

Two verbs came with it, and both were owed. `Command::New` used to mean "another document of the
kind already open", with a comment saying a New Spreadsheet / New Document *pair* was a menu
question left open; it is now that pair (Ctrl+N and Ctrl+Shift+N). And `File ▸ Welcome Screen`
goes back, because a welcome screen you can only ever see once is a splash screen.

`--sheet` and `--text` still skip it, which is what keeps `grind-win32 --text` meaning exactly
what it meant; `--render-to` with no file draws it, which is how it is looked at without Windows.

### 12. Every dialog is one shape — ContentDialog's — and **it fits the screen**

Until W14 this shell had five hand-built popups (a prompt, a chooser, the filter, the chart
preview, the print preview) and `MessageBoxW` for the rest, and the complaint that ended it was
that dialogs did not fit: content ran off the screen. Reading them showed why, and it was the same
three mistakes five times over. Each asked for a fixed **outer** size in logical pixels — the print
preview's 640 × 860 is 1075 pixels tall at 125%, taller than a 1080p work area — centred itself on
its owner with **nothing stopping it at the monitor's edge**, and placed its buttons by **guessing
the caption's height** (`h - button - 40`), which is a different number at every scaling.

So there is now one modal, and two rules about it:

- **The size is the content's, the bound is the screen's.** A caller describes its *body* — a
  message, a field, a list, a reader, a chart, the pages — and `run` measures it in the fonts it
  will be drawn in, before there is a window. `modal::wanted` turns that into a window size, and
  `modal::place` clamps it to the **work area of the monitor the owner is on**, less a margin, and
  centres it on the owner only as far as that allows. What does not fit is the body's to absorb: a
  list or the reader scrolls, a chart or a page scales. The print preview asks to be as tall as the
  screen and as wide as the page is at that height. All of it is a portable function, swept over six
  scalings and three owner positions by `a_modal_never_leaves_the_work_area`, and checked under Wine
  at 150% on a 1280 × 720 screen.
- **The anatomy is ContentDialog's.** No caption — the caption and frame exist only so the
  compositor gives the surface its shadow and Windows 11's rounded corner, and `WM_NCCALCSIZE`
  hands the whole window to the client — then a title in the type ramp's *Subtitle* (20, semibold,
  *Segoe UI Variable Display*), the body on the lighter layer, and a **footer** on the window's own
  ground under a hairline, 24 pixels of padding throughout. Answers alone share the footer in equal
  columns, at least two, so a lone *Close* takes the trailing half; anything that is not an answer
  (*Previous*/*Next*, a chart's kinds, a filter's *Clear*) stands at the leading end and the
  answers shrink to their words. One button is the **default**, drawn in the accent and meant by
  Enter — and where an answer destroys something the default is the safe one, so *Delete Budget?*
  has its accent on **Cancel**. The window behind is dimmed with Fluent's smoke (black at 30%) for as
  long as the modal is up, except under Wine, which can draw a layered window's alpha only with a
  compositor running and would draw it opaque.

What is inside is drawn by this shell rather than handed to the system, which is what closes W10's
four light rectangles. Buttons are `BS_OWNERDRAW` with a thin subclass that tracks the pointer —
a button that does not answer hover is what most gives a custom-drawn window away — and tells the
dialog manager the focused one is a push button, so Enter on a focused *Cancel* means Cancel. Lists
are `LBS_OWNERDRAWFIXED`: Fluent's ListView row, a subtle rounded ground under the pointer and the
selection, an accent pill on the selected row, a check box in place of the pill for the filter, and
a tab in a row splitting it into two columns (the shortcut list's key against the trailing edge). A
field is an `EDIT` with no border of its own inside a drawn TextBox — a control's ground and stroke,
the strong line along the bottom turning into the accent, two pixels thick, while it has the
keyboard. `MessageBoxW` is replaced by the same modal with the **verbs on the buttons** — *Save*,
*Don't Save*, *Cancel*; *Delete*, *Cancel* — which is what Windows' own applications say and what
*Yes*/*No* never did. Only `main.rs`'s message before any window exists is still a `MessageBoxW`,
since there is no window yet to theme one after.

The last piece is the **application manifest** (`ui_win32/data/grind.manifest`, linked by
`grind.rc`): Common Controls 6, so what Windows still draws — the edit's caret and selection, the
scroll bars, the popup menus, the file dialogs' own controls — is the current control rather than
Windows 2000's; per-monitor DPI v2 from the loader rather than only from `SetProcessDpiAwarenessContext`
at run time; and the Windows 10/11 compatibility id. `artifacts.yml` reads it back off the linked
`.exe` beside the icon and the version block, since `embed_resource` silently embeds nothing where it
finds no resource compiler — which is every machine but `windows-latest`, this one included.

The first body built on the shape after it was finished is **alt text** (Format ▸ *Picture Alt
Text…*, and the text pane's context menu over a picture): `dialog::alt_text`, a body with **two**
fields where every other has at most one — the picture decoded by WIC on the window's own ground,
the short field, `grind_text::picture::advice` and a count redrawn under it on every `EN_CHANGE`,
and the long description, a multi-line `EDIT` with no scroll bar of its own — whose rows are
`modal::alt_body`, tested on Linux, and which gives up the picture's height first when the work
area is short. Each field keeps its own focus, so each frame wears the accent only while it has the
keyboard. Seen under Wine; it found one bug a review would not have — `SetWindowTextW` fills the
field before the modal knows which control it is, and the `EN_CHANGE` that fires then reset the
count to nought.

### 13. The menu bar is **drawn**, in the band Windows' own would take

Decision 4 is unchanged: the menu bar is this platform's growable surface. What changed is who
draws it. `user32`'s bar is a strip of `COLOR_MENUBAR` with a hairline under it that no palette
chose, it stays light over a dark window, and there is no documented way to change either — so the
window read as a W10 surface under a 1995 one.

`menubar.rs` draws it instead, and the choice that kept this cheap is *where*: **in the non-client
area**, directly above the client rectangle, exactly where the system's bar would be.
`WM_NCCALCSIZE` takes the band off the top of the client, `WM_NCPAINT` paints it, `WM_NCHITTEST`
calls it `HTMENU`. Every pane's own layout, hit-testing and child-control placement is in client
coordinates and none of it moved. The look is Fluent's *MenuBar* in its compact size: the window's
own ground (the caption is already painted in it, so caption, bar and format strip are one
surface), titles in Body type, and a subtle rounded ground under the pointer, under the open menu
and under the keyboard.

What the system bar did for free is rebuilt on documented API, and each piece is tested or was
driven under Wine:

- **The popups are still Windows' own** — `TrackPopupMenuEx` over the same `HMENU`s `build_menu`
  always made, kept rather than attached — so every item, accelerator label, check mark, mnemonic and
  `WM_INITMENUPOPUP` check inside a menu is exactly what it was.
- **Moving between menus**: Left and Right inside an open menu, and the pointer crossing onto another
  title, close it and open the neighbour — a `WH_MSGFILTER` hook for the length of the menu loop, the
  standard way a drawn bar does this. A press on the open menu's own title closes it.
- **The keyboard**: Windows still turns Alt, F10 and Alt+letter into `SC_KEYMENU` for a window
  with no bar of its own, and the drawn one answers it. Alt or F10 alone walks the bar (Left, Right,
  Enter, Up, Down, a title's letter, Escape, Alt again) in a message loop of its own **without moving
  the focus**, since moving it would commit a half-typed cell; Alt+letter opens that menu with its
  first item highlighted; Escape out of a menu opened from the keyboard returns to the bar. Mnemonic
  underlines are drawn only while the keyboard drives, Windows' own rule. Alt+Space is still the
  window menu. `every_title_in_the_bar_has_a_mnemonic_and_no_two_share_one` holds the titles to it.
- **Dark popups.** The menus themselves are the one thing this shell asks of Windows through API it
  does not document: `uxtheme`'s `SetPreferredAppMode` (ordinal 135) and `FlushMenuThemes` (136),
  and the `DarkMode_Explorer` theme class for scroll bars — the calls Notepad, Explorer and the
  Settings app make, stable since Windows 10 1809. They are looked up rather than linked and skipped
  below that build (`theme::dark_controls`), so a Windows without them gets the light menus it had.

The cost, named: the bar is no longer `user32`'s, so **UI Automation sees no menu bar** — a screen
reader reaches every verb through the popups and the accelerators but not the bar itself as a list of
menus. That joins `accesskit_windows` on the deferred list.

## The crate

A `*` marks what W0 through W3 have built; everything else is the plan.

```
ui_win32/
  Cargo.toml            * grind-win32; the `windows` dependency target-gated on cfg(windows)
  build.rs              * W8's icon and version resource, attempted only when the target is
                          Windows and a no-op everywhere this crate is checked from Linux
  data/
    grind.rc            * the version block and the `1 ICON "grind.ico"` line `build.rs` compiles
                          — both named with a *literal*, which is load-bearing: see W8 below
    grind.ico           * generated from grind.svg (`magick -define icon:auto-resize=...`), not
                          built by this crate — turning an SVG into an ICO needs librsvg
    grind.svg           * the source: the two GTK apps' own icon language, one mark
    grind.manifest      * W14: Common Controls 6, per-monitor DPI v2, the Windows 10/11 id —
                          linked by grind.rc as resource 1 of type 24 (decision 12)
  src/
    main.rs           *   argv, kind sniff, a message box for errors before a window exists
    args.rs           *   the command line as a pure function (W0)
    win.rs           [W]* the class, the wndproc, the message loop, GWLP_USERDATA, the two
                          child EDITs (name box and editor), the menu bar, the verbs, and the
                          windowless render path
    gdi.rs           [W]* RAII wrappers for fonts and brushes; the back buffer; and the DIB
                          target behind --render-to, with its BMP writer (W2)
    metrics.rs       [~]* Metrics + Faces over a memory DC, with the font cache — and, portable
                          beside it, which face a block is set in and the per-code-unit ->
                          per-char fold decision 3 turns on (W5a)
    theme.rs         [~]* W10: Fluent's three ramps — colour (three grounds), type and space —
                          the accent tinted until it reads, `automatic_ink`, and the control
                          states; all portable, incl. the selection wash's `Rgb::blend`. The
                          registry read and the window chrome (dark title bar, caption colour,
                          rounded corner) are the [W] half
    menu.rs           *   the menus as data, the accelerators, and the command-id table
    menubar.rs       [~]* W14: the drawn menu bar — its layout, mnemonics and the keyboard's walk
                          portable; the non-client band, the popups and the message filter [W]
    modal.rs          *   W14: ContentDialog's anatomy, and `place`, which fits a modal to the
                          work area of the monitor its owner is on
    notice.rs         *   every sentence the notice bar says, as a pure function
    dialog.rs        [W]* every modal: the file dialogs, and one drawn modal (W14) for the
                          questions, messages, prompt, lists, filter and the two previews —
                          its buttons, rows and fields owner-drawn, the owner dimmed
    sheet/
      geom.rs           * pixels <-> cells, prefix sums over the document's own widths, the
                          strip, and which visible track a cursor may stop on
      keymap.rs         * virtual-key codes -> a motion; the selection; the Ctrl+arrow rule
      status.rs         * the name box, the formula bar, and the status bar's aggregates, all
                          over a real `App`
      state.rs          * Ready / Enter / Edit, the editing state machine, and the two
                          conversions an edit needs (display syntax, and UTF-8 bytes <-> the
                          UTF-16 units EM_GETSEL/EM_SETSEL count, both ways — the latter
                          `grind_core::utf16` since the Mac's M1, in the control's `i32`)
      assist.rs         * W9: what to offer somebody typing a formula and what the call under
                          the caret wants next, over `grind_sheet::formula::assist`; the runs
                          the band draws; the keys a list of offers claims; and the function
                          list's rows — all portable, none of it aware of a window
      draw.rs        [~]* a viewport painted onto an HDC — and, portable beside it, what a cell
                          *looks like*: alignment, weight and colour resolved from `CellStyle`,
                          and what the selection wash does to it
    text/
      geom.rs           * where each block sits down the page, the window's bands, and the whole
                          document measured into a `Flow` — portable, and checked on Linux
                          against what `grind text view --width` breaks (W5a)
      keymap.rs         * a key -> grind_text::caret::intent_for, Ctrl+A, Windows' word rule
      status.rs         * what the status bar says: what the caret is in and the document's length
      draw.rs        [~]* laid-out blocks painted onto an HDC, run by run — and, portable beside
                          it, how a line is cut into runs, which part of it is selected, and the
                          two characters that are measured and never drawn
    code.rs           *   the "Show Source" list — every line of `App::project`'s text
                          form, over a modal `dialog::choose` rather than a drawn pane (W6)
    problems.rs       *   the "Check Document" list — every `App::lint` finding, worst
                          first, the same way (W6)
    welcome.rs       [~]* W11: the welcome screen — the three choices and what each is called,
                          where the cards go, which one a point is in and which one a key moves
                          to, all portable; only putting pixels down is the [W] half
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

## File associations — offered, never taken

W8 asked for this **written down**, and decision 1 already gives half of it: Windows associates
files by ProgID rather than by MIME type, and one executable registers as many ProgIDs as it
likes, each with its own icon, description and verb — which is why `grind-win32.exe` is one
binary for both document types where the two GTK shells are two processes. The rest of the
answer is which ProgIDs, and — since X6 made this shell open Excel workbooks — **how hard to
claim them**. `assoc.rs` is both, as data:

| ProgID | Extension | Verb | Icon |
|---|---|---|---|
| `Grind.Spreadsheet` | `.fods` | `"…\grind-win32.exe" "%1"` | the executable's, index 0 |
| `Grind.SpreadsheetPackage` | `.ods` | the same | the same |
| `Grind.Document` | `.fodt` | the same | the same |
| `Grind.DocumentPackage` | `.odt` | the same | the same |
| `Grind.Projection` | `.grind` | the same | the same |
| `Grind.Workbook` | `.xlsx` | the same — imported as a new, unsaved ODF document | the same |
| `Grind.MacroWorkbook` | `.xlsm` | the same — the macro counted, never run | the same |
| `Grind.WordDocument` | `.docx` | the same — imported as a new, unsaved ODF text document (`doc/docx-import.md`) | the same |
| `Grind.MacroWordDocument` | `.docm` | the same — the macro counted, never run | the same |
| `Grind.Csv` | `.csv` | the same — a new, unsaved ODF document, the delimiter read from the file | the same |
| `Grind.Tsv` | `.tsv` | the same | the same |

The flat and the package form of each ODF type get their own ProgID rather than sharing one,
which is what lets a future icon tell them apart; `.grind` is one ProgID for both applications,
because its own first line says which document it is. The two Word rows are compiled out with
the crate's `docx` feature for the same reason as the Excel ones. The two Excel rows are compiled out with
the crate's `xlsx` feature — offering a type the binary then refuses is worse than not offering
it — and `.xls`/`.xlsb` are absent because the filter does not read them. A CSV takes the
workbook's path exactly (`grind_sheet::csv::open`); `.tab` opens but is not offered, since
nothing on Windows calls a file that. Excel is usually `.csv`'s handler on Windows, and stays it.

**Offered, never taken.** Per extension, the ProgID is added under `.ext\OpenWithProgids`, which
puts Grind in Explorer's *Open with* menu and in the "How do you want to open this file?"
chooser; `Applications\grind-win32.exe\SupportedTypes` says the same from the executable's
side; and the set is published as `Software\Grind\Capabilities` under
`RegisteredApplications`, which lists Grind with every type it opens in Settings > Apps > Default
apps. **The extension's own default value is never written**, and nothing under `UserChoice`
ever is — a test holds the first for every type. So whatever opens a double-clicked `.xlsx` today
(Excel, LibreOffice, nothing) still does after Grind has run, and making Grind the handler is the
user's one click in a dialog Windows owns. Where nothing handles a type at all — a `.fods` or a
`.grind`, usually — Windows asks on the first double-click and Grind is in the list.

**No installer, so the window registers itself.** Everything is per-user
(`HKEY_CURRENT_USER\Software\…`: no elevation, nothing another account sees), and every start of
the window checks each ProgID's command against this executable's own path and writes the offer
only when one is missing or names another copy (`assoc::needed`) — so an ordinary launch touches
no key, and an `.exe` run from the test disc, copied out of a download or moved is re-offered
from where it now is rather than leaving a double-click that fails. `SHChangeNotify` follows
each write so Explorer's menus catch up without a sign-out. `--render-to` never registers: a
frame is a CI step, not a person. `--register` does it and exits, for a script;
`--unregister` removes exactly what `--register` writes — whole keys where the key is ours, one
value where it is shared (`.xlsx\OpenWithProgids` belongs to everybody who opens one) — which a
second test holds in both directions.

**Measured under Wine**, with the caveat that matters: every value lands where the table says,
a pre-existing `.xlsx` default (`Excel.Sheet.12`) survives registration and `--unregister`, a
hand-deleted ProgID is repaired on the next start, and the registered command line opens a
workbook whose path has a space in it straight into the import. That last step needed the same
keys copied into `HKEY_CLASSES_ROOT` by hand, because **Wine does not merge
`HKCU\Software\Classes` into `HKCR`** the way Windows does — a Wine limitation, not this shell's,
and the reason the per-user half is only provable on a real Windows.

**An Excel workbook opens** (`doc/xlsx-import.md`, X6) from File ▸ Open — an *Excel Workbook* filter beside the ODF one — or the command line: the caption is `*budget.fods`, the notice bar carries the report, and Save opens Save As in the workbook's folder — sniffed from the bytes,
imported through `grind_xlsx::open`, and opened as a **new, unsaved ODF document with no path**
under `grind_xlsx::suggested_name` (`budget.xlsx` → `budget.fods`), with the report's one
sentence (`Report::summary`) shown. Having no path is the point: one way in and never out
(`doc/not-doing.md` §1), so nothing can write ODF over the workbook it came from. `grind-open`
decides it, the same in every shell, and the crate's `xlsx` feature compiles it out.

## What it will not do

R10 allows per-shell feature gaps and requires them to be named. These are the named ones.

**Wrap and borders, on the grid — closed 2026-10-02.** They were left off the strip on purpose
while the grid drew neither: a toggle whose effect cannot be seen is worse than none. The grid now
draws both — borders through `grind_sheet::look::border_strokes`, wrapped text through
`grind_core::layout::wrap` over GDI measurements (`sheet/measure.rs`) with rows grown by
`grind_sheet::autoheight` — and Format has *Wrap Text*, *All Borders* and *Remove Borders*
(menu verbs, not strip buttons; the strip keeps its admission test). `dashed` and `dotted` borders
draw solid. A decimal count
beyond one step at a time, a grouping toggle and a currency other than `numfmt::CURRENCIES`' three
are `grind sheet format`'s; this strip's number picker is one click per kind.

**A colour picker with no colours in it.** Both panes' *Text Colour*, *Highlight* and *Cell
Background* open `dialog::choose` over the palette's **names** — capitalised now, and one list
(`strip::colour_choices`) — with no swatch beside each. An owner-drawn `LISTBOX` row would carry
one; it is the next thing this chooser wants, and the reason it is named here.

**Not drawn, kept intact.** A **chart** in a file is read, kept and written back untouched, and
nothing here draws one — the same position `grind-tui` takes, and it is a deliberate stop rather
than a stub. A **table** in a text document used to be the second, and is not
any more: it is drawn as a grid (the UX pass section above). What this pane still cannot do to
one is *make* it — no Insert Table, which the CLI, `grind-text-gtk` and `grind-tui` have — or
merge cells or set a column width, which nothing in the suite can.

**Images are decoded and drawn now, not just kept.** `image.rs` is WIC — `IWICImagingFactory`
over an `IStream` `SHCreateMemStream` wraps the bytes in, a format converter to premultiplied
`32bppPBGRA`, `CopyPixels` into a buffer `gdi::blit_image` composites with `AlphaBlend` — the
Windows answer to the question `ui_text_gtk`'s `texture_of` asks gdk-pixbuf, catching this shell
up with that one rather than staying a named gap. Ctrl+Shift+I or Format ▸ Insert Picture… opens
`dialog::open_image_path` and reaches `App::insert_image` the same way `Ui::embed_image` does: an
empty block is used as it stands, anything else gets a fresh paragraph after it, and the picture
is fit to the column, aspect kept, never larger than its own pixels — `grind_text::picture_of`,
hoisted out of `ui_text_gtk/src/view.rs` the same day `format::Change` and `indent_kind` were, so
"is this block a picture, optionally with a caption" is one answer both shells share. A caption
(the run of text after the image, in the same block) is measured and drawn with `DrawTextW`'s own
word wrap rather than the core's line layout — the one text in this pane drawn outside decision
3's rule, and on purpose: nothing puts a caret inside a caption, so there is no layout for GDI's
wrapping to disagree with. `text::geom::flow_of` takes a picture-height hook now instead of
knowing about images itself, which is what keeps that function testable with no Windows and no
decoder at all (`&|_, _| None` is what a Linux caller, or a document with no pictures, passes).
Two things are true of every shell alike and not just this one: an image sitting *mid-sentence*
still draws as the placeholder character (`\u{fffc}`), because nothing here lays inline content
out around one yet, and `svg:width`/`svg:height` are not honoured — "fit the column" is the
answer every simple viewer gives instead, `ui_text_gtk`'s included.

**Not built, though L3 no longer stands in the way.** Wrapped cells and **row auto-height** are
still this shell's own gap: `ui_sheet_gtk`'s row auto-height measurement moved onto
`layout::Metrics` (L3), which was the missing precedent rather than a missing core capability,
so building the same pass here would mean the `metrics.rs` `Metrics` impl this shell already has
for the text pane, a second call site, and nothing new in `grind-core`. Until it is built, a
document's *stored* row heights are honoured and no row is measured from its contents.
**Pagination**, **printing** and **RTL** are gated in `doc/not-doing.md` and
`doc/text-layout.md` and are not this shell's to open.

**Deferred by decision, reachable from the CLI (R9).** A hidden row or column is drawn as
**gone**, with none of `ui_sheet_gtk`'s marker straddling the boundary — so this shell shows what
the document says and offers no way to *unhide* from the grid. `grind sheet hide --unhide` does
it, which is the R9 answer. W2 did not add the marker: the cursor now steps *over* a hidden
track (`keymap::onto_visible`, above), which was the urgent half, and a hit-test target that is
one pixel wide is a chrome question rather than a navigation one. No **autoscroll on a drag**
past the edge of the window — a drag stops at the last visible cell, and Shift+arrow, which does
scroll, is the way to select further. No **zoom** before W8. No **point mode** — arrow keys
building a reference into a half-typed formula, the one piece of the GNOME
window's formula entry (`doc/sheet-shell.md`) this shell does not have. ~~No autocomplete or signature hints~~ — **built in W9**, as a drawn
band rather than a popup (`sheet/assist.rs` says why), over the pure half hoisted into
`grind_sheet::formula::assist` so this window and the GNOME one cannot disagree about which
argument the caret is in. **The autofilter is built**: Data ▸ *Autofilter* is the on/off switch
`sheet.filter` is in the web shell (`toggle_filter`, no dialog — the selection becomes the
range), a dropdown button is drawn on every cell of the filter's heading row
(`GridGeom::filter_button`, `ui_sheet_gtk`'s own mirrored), and clicking one opens
`dialog::choose_multi` — a multi-select `LISTBOX` rather than an owner-drawn checkbox list, so a
plain click toggles a row with no `WM_DRAWITEM` pair to get there — to pick which of the
column's values stay, with *Clear* as a third button dropping the condition entirely. **CSV is built, both directions**, and it is where this shell's File menu stops being universal:
*Import CSV…* and *Export CSV…* sit in their own section there, and `applies_to` leaves both out
over a text document, so that pane's File menu is the four file verbs and Exit. An import is a
file dialog and nothing else — `csv::Import::sniffed`, the fields landing at the **cursor**, one
undo step, and the notice bar saying how many cells and where (`notice::imported`); an export is
a save dialog whose two filters are the whole of the delimiter question, since the *name* decides
it (`csv::Dialect::for_name`), and it writes a file without touching the document, which is what
its own notice says instead of naming a key (`notice::exported` is this file's one sentence with
no way out in it, because there is nothing to undo). Both dialogs are opened with no borrow held
across them, decision 7 as usual. The seven options `grind sheet import-csv` carries are a named
gap here as in every window. Data ▸
*Format as Table* still has no dialog of its own: fixed defaults (a header, no totals row),
the same zero-prompt shape `sheet.filter` has in the web shell, but now applies a *real*
filter rather than an empty one with buttons and no conditions. Beside it, *Format as Table
with Totals…* is that composite having first asked which aggregate the totals row carries —
`dialog::choose` over `TotalsFunction::ALL`, the listbox this shell already opens for every
other one-of-a-short-list question, and a second verb rather than a prompt on the first so the
plain one keeps its zero-prompt shape. **Find and replace over cells** are the Edit menu's:
*Find…* (Ctrl+F) asks for a word through `dialog::prompt` and lands on the first cell holding it
at or after the cursor, *Find Next* and *Find Previous* (F3, Shift+F3 — every Windows program's)
step on across every sheet and wrap, and *Replace…* (Ctrl+H) asks what and with what and calls
`App::replace` over every sheet in one undo step. Where it landed and what a replace did are the
notice bar's (`notice::found`, `notice::replaced`), since that bar already says what an import
did; a formula the replace would break is left alone and named there. Every match is
`App::find`'s and every next hit `grind_sheet::find::step`'s, the GNOME bar's and the browser's
own. Prompts rather than the system's `FindTextW` dialog, for decision 7's reason: that dialog
is modeless, and a second window with its own message routing is exactly what this shell keeps
out of `win.rs`. Named gaps: no match-case or whole-cell control (the CLI has both), no marking
of every hit. **The text pane has the same three verbs since 2026-10-02**: Find…, F3/Shift+F3 and
Replace… over `grind_text::find` (case-blind hits, selected so typing replaces one) and
`App::replace` (exact, one undo step), the sentences in `notice.rs`.
**Conditional formatting** is Data ▸ Add Conditional Format… (`dialog::prompt` for the
condition, `dialog::choose` for the look) and Remove Conditional Format… (`dialog::choose` over
the sheet's rules) — the banding *Format as Table* paints is static cell styling, not a rule. No **command palette**, by
decision 4.

W3 adds three of its own, each smaller than it sounds. There is no **recent-files list**, where
`ui_sheet_gtk` has `gtk::RecentManager`: Windows' equivalent is `SHAddToRecentDocs` plus a
registered ProgID — which now exists ("File associations"), so this is the next thing that can be built on it. There
is no **greying of unavailable verbs** — Undo is enabled with nothing to undo, and pressing it
does nothing rather than something wrong — because `MF_GRAYED` means tracking menu state on
every change, and W7 is where the menus are finished. The **sheet was chosen from a menu
rather than from a tab strip** until the UX pass below, on a reading of `doc/sheet-shell.md` that
was wrong: the strip that document removed for looking like a ribbon was its *tool* row's
Format/View/Calculate tabs, and the GNOME window has sheet tabs to this day. Tabs are built now.

**System-drawn, and therefore not themed by us — narrowed in W10, closed in W14.** W10 left four
light rectangles in a dark window — the push buttons, a list's selection bar, the menu bar and the
message boxes — and W14 drew all four (decisions 12 and 13). What Windows still draws is what it
should: the file and print dialogs, which are the system's own and follow its theme; an `EDIT`'s
caret and text selection; the scroll bars, themed dark through `DarkMode_Explorer` from 1809 on; and
the popup menus, dark through the two `uxtheme` ordinals decision 13 names. **A dialog cannot be
dragged** — ContentDialog cannot either, and the owner is dimmed and waiting — and **the drawn menu
bar is not a UI Automation menu bar** (decision 13).

**W10's own gaps.** The **system accent is followed but not its ramp**: Windows publishes
`AccentPalette`, six precomputed tints, and this shell derives its own from `DWM\AccentColor`
instead — an undocumented binary layout against arithmetic that can be tested, and the arithmetic
won. If the two ever visibly disagree on a real machine, the fix is to read the palette and keep
the contrast guard on top of it. **`--dark` is a flag, not a preview**: the *window* still follows
the registry, so a developer cannot flip the running window's theme without changing the machine's
(what W10 did instead was a throwaway Wine prefix with the two registry values set, which is
written up in the verification section). **Hover is the text pane's only interaction state** — the
grid's header buttons and its corner do not light up under the pointer, which they should, and the
sheet's strip has no controls to light up. And there is no **focus ring** on anything drawn: the
`focus`/`focus_inner` tokens exist and nothing paints them, because nothing drawn in this window
takes the keyboard yet (the two `EDIT`s draw their own).

**The toolkit's own limits.** No **Mica**: `DWMWA_SYSTEMBACKDROP_TYPE` is reachable and shipped,
but a backdrop only shows through pixels the application does not paint, and a GDI window that
fills its client area with an opaque brush paints all of them. *This is reasoned rather than
measured*, and it is flagged as such in the sibling repository too. The dark title bar
(`DWMWA_USE_IMMERSIVE_DARK_MODE`) is the part that survives and is implemented. No **Fluent
controls**, which are WinUI's, and no v6 **toolbar**, which would draw itself in the system's style
rather than this window's (decision 4). No **shaping** at all
while `metrics.rs` is GDI's: decomposed text, ligatures, emoji sequences and the LTR complex
scripts (Devanagari, Thai, Khmer) are each drawn as separate boxes. Decision 3 ranks these by
likelihood and names the trigger; the LTR complex scripts in particular are a gap of this
shell's own making, **not** something `doc/text-layout.md`'s RTL exclusion already covered.

**Accessibility.** A painted GDI window exposes no UI Automation tree. The floor is the
**system caret**, and it is built: `place_system_caret` calls `CreateCaret`/`SetCaretPos`/
`ShowCaret` at `Text::caret_geometry` — the same geometry `text/draw.rs` draws its own caret
from, sized to the caret's own line so a heading's caret is taller than a paragraph's —
whenever `WM_SETFOCUS` arrives, whenever `text_refresh` runs (which is every motion and every
edit), and once more after every `WM_PAINT`, which is the choke point that catches the handlers
that invalidate the window directly rather than going through `refresh`. `WM_KILLFOCUS` calls
`DestroyCaret`, so a modal dialog stealing focus — or another application entirely — does not
leave one behind for whatever Windows shows next to trip over. The pairing the accessibility
section used to name as the reason this was not free is built too: `paint` calls `HideCaret`
before the back buffer's `BitBlt` and repositions-and-shows it after, because the blit knows
nothing about the caret and paints over it otherwise. `render`'s windowless frame is the one
exception — it has no `HWND` and so no system caret, which is why it still draws the manual
rectangle `draw_text_frame`'s `system_caret` flag suppresses everywhere else, and why the caret
in a `--render-to` `.bmp` is always on rather than mid-blink: two renders of one document have to
be byte-identical, and Windows' own blink phase is not something this process controls.
Verified interactively — screenshotted under Wine and Xvfb, since a caret is exactly the kind of
bug `cargo check` cannot see: a first pass left `text_button_down`'s direct `InvalidateRect`
call unrepositioned, and the caret stayed at its old address after a click until whatever next
touched `refresh` moved it, which is what `place_system_caret`'s call from `paint` itself (rather
than only from `text_refresh`) exists to close off for every such handler at once rather than one
at a time. `accesskit_windows` remains the fix if a full UI Automation tree ever matters, and
naming it is still not the same as planning it — the system caret is the floor, not the ceiling.

**W5a's own gaps, and most of them are now closed.** `App::type_markdown` **has since landed**:
every character typed into the pane goes through it rather than through a plain `insert_text`, so
`**bold**` is read as it is typed exactly as `ui_tui`'s is, with the same `resume` field carrying a
completed span's style across the keystroke that closes it. **The format strip has landed too**:
a band under the banner and above the body (`text::geom::Page::strip`/`strip_buttons`), three
buttons drawn by `text::draw::paint`'s own `draw_strip` and pressed in — `theme.header_active`'s
wash — when `App::char_style` over the selection already agrees, or, with nothing selected, when
the style the *next* character typed would carry does (`resume` if a markdown span left one
pending, otherwise the character just behind the caret — `text_style_here`). A click hit-tests the
three rectangles in `text_button_down` before the caret-placement fallback runs, the same
precedence `button_down`'s own `Strip` match gives the grid's name box and formula bar, and every
path converges on the one `text_emphasise` function: the click, Ctrl+B/I/U, and the Format menu
all toggle through it, over the same logic `ui_tui::emphasise_selection` uses (on across the whole
selection, or off when it already agrees). **The strip now carries all five of
`markdown::Emphasis`** — Strike and Code joined Bold, Italic and Underline as drawn toggles rather
than staying menu-and-key-only, catching this pane up with `grind-text-gtk`'s own bar — and four
more controls beyond the toggles: Family and Size open `dialog::choose` (`text_pick_family`,
`text_pick_size`), the two swatches (`text::geom::Page::strip_color`/`strip_highlight`, filled with
`gdi::round_rect` at radius zero, hollow for *Automatic*) open the same popup over
`grind_core::style::PALETTE`, and *Clear* is one shot through `grind_text::format::Change::Clear`.
`grind_text::format::Change` — the whole vocabulary of what a control writes, `Bold`/`Family`/
`Clear` and the rest, tested with no display — is hoisted into `grind-text` itself rather than
copied, the same move `sheet/src/formula/assist.rs` made when `grind-win32` wanted the same
formula-typing answers a second shell already had; `ui_text_gtk/src/format.rs` now re-exports it
instead of defining its own. The families this picker offers are **curated, not enumerated** —
eight common Windows faces plus the document's own when it set one none of them name — because
this shell has no `EnumFontFamiliesExW` wiring yet, which is a named gap rather than an oversight;
`grind-text-gtk`'s font drop-down lists every family Pango can resolve. `text_format` is
`text_emphasise`'s general form, applying any `Change` to the selection's `CharStyle` the same way;
`text_button_down`'s hit test answers a `text::geom::StripHit` now rather than a bare button index,
since the strip has controls that are not toggles. **Go-to has since landed too**, as F5 or
Ctrl+G (the grid's own two keys for the same verb) opening a modal prompt rather than a strip
box — this pane owns no child control to put one in — over `loc::parse` and `App::resolve_caret`,
the same two calls `ui_tui`'s `cmd_jump` makes; `p12` / `#intro` / `§2.1.3` are all addresses it
now understands. **A block's kind can be changed too**: the same Format menu's Paragraph and
Heading 1/2/3 items, and `ui_text_gtk`'s own Ctrl+0/1/2/3, reach `App::set_kind` at the caret's own
block — no selection needed, unlike the three toggles above. **Title and Subtitle joined them**:
`text_set_kind` now takes an optional named style alongside the `BlockKind`, and
`grind_text::named_style_for` — the rule that a style is only ever taken off when a shell put it
on, hoisted out of `ui_text_gtk`'s own `Ui::set_kind` the same way `indent_kind` was — decides
whether Paragraph clears `Title`/`Subtitle` or leaves a document's own name (`Quotations`, say)
exactly as it found it. **Tab now nests a list the same way it does in `grind-text-gtk`**:
Shift+Tab is told from Tab (`grind_text::caret::Intent::Tab`), and `Editor::tab`
answers `grind_text::indent_kind` — nest one level deeper, un-nest one, or start a list at the
front of a block — before falling back to a literal `text:tab` character, exactly the rule
`ui_text_gtk`'s `Doc::indent` already had and now shares rather than repeats. **The outline dialog
has landed too** — Ctrl+Shift+O or Edit ▸ Outline, a modal list built on the built-in `LISTBOX`
control rather than a strip box (this shell's own popup type, `dialog::choose`, generic over
strings so `dialog.rs` stays ignorant of `Heading`), one row per heading indented by depth and a
double-click or OK jumping straight to its block. **A level past 3 and a list item both reach
this window too now**, over the same `dialog::choose` popup: Ctrl+Shift+K or Format ▸ Block
Kind… lists Paragraph, Heading 1 through 6 and List item at four depths, and picking one is
`App::set_kind` at the caret's block exactly as the three quick keys are. `doc/text-shell.md`
names "no lists UI" as a gap in both GTK windows and the browser; this is the first window in
the suite where a paragraph can be turned into a list item at all rather than only read back
looking like one. The clipboard carries
**plain text only** — one `\r\n` per block, no
`CF_RTF` and no `HTML Format` — so formatting does not survive a copy into WordPad. **The menus
now do know which pane they are over**: `menu::applies_to` is the portable half — tested on Linux
with no window at all — of which verb means anything for which document kind. It was first
reached through `EnableMenuItem`'s `MF_GRAYED`, and that did not last: the sheet's own six verbs
so outnumber the universal ones that `&Sheet` and half of `&View` sat on screen, greyed, on every
document that was not a spreadsheet, and the bar read as a grid that had not noticed it was
showing something else. `menu::items_for`/`menu::menu_has_items` **omit** instead — `&Sheet` and
`&Data` are missing from the bar entirely over a text document, `&Format` is missing over a
spreadsheet, and `&Edit`/`&View` each lose the one item that does not apply (`Outline`; `Cell
Roles`) while keeping the rest — rebuilt whenever `adopt` changes which kind the window holds,
same as greying was. What W7 still owes is state *inside* one kind — Undo greyed with nothing to
undo, Paste greyed with an empty clipboard — which needs the bar rebuilt on more than a pane
change and is a different piece of work from telling the two document types apart.

**Surrogate pairs and one corner of the IME have landed too.** `WM_CHAR` carries one UTF-16 code
unit per message, so any character outside the Basic Multilingual Plane — an emoji, most of the
rarer CJK ideographs, some IME output — arrives as two consecutive messages; `surrogate.rs`
(portable, tested with no window) is the pure half that reassembles a pair, and `typed_char` is
where the pending high half waits, per pane rather than in a static because two windows must not
share it. `WM_IME_STARTCOMPOSITION` is now the one IME message this shell answers at all: it
positions the composition window Windows draws for itself — `ImmSetCompositionWindow` at the
caret's own pixel position, computed the same way `text/draw.rs` places the drawn caret — and
hands everything else about composing back to `DefWindowProcW`, which is what a native `EDIT`
control gets automatically and this custom-drawn pane does not. The sheet needs none of this once
an edit is open (the `EDIT` positions its own IME), so the surrogate state is per pane and the
composition positioning is text-only. What that one message does **not** buy: no inline
composition string drawn in the pane's own ink — the IME's default floating box does that
instead — and nothing has been driven under a real IME, since Wine ships none to test against;
`artifacts.yml`'s `windows-latest` runner is the first place this can be watched working for real.

**The welcome screen has no recent-files list, and that is a decision rather than an omission.**
LibreOffice's Start Center leads with one, and it is the obvious fourth thing to put under the
three cards. It is also a *store* — a list of paths this application would have to write
somewhere outside any document, read back at startup, prune when a file has moved, and answer
privacy questions about — and this shell has no such store at all today (no settings, no session,
no registry key of its own but the theme it reads). `ui_sheet_gtk` gets recent files for nothing
because `gtk::RecentManager` is the desktop's list rather than the application's; Windows has an
equivalent in `SHAddToRecentDocs` and the Jump List, and reaching it is the shape the answer
should take when it is built — the system's list, not a file of our own. Until then the three
cards and File ▸ Open are the whole of it. Templates are the same question one step further out
and are gated with the rest of them in `doc/not-doing.md`.

## Verification

**On this Linux machine, and in CI on an Ubuntu runner:**

```sh
rustup target add x86_64-pc-windows-msvc          # already installed here
cargo check   -p grind-win32 --target x86_64-pc-windows-msvc
cargo clippy  -p grind-win32 --target x86_64-pc-windows-msvc --all-targets -- -D warnings
cargo test    -p grind-win32                      # the portable half: geom, keymap, state,
                                                  # status, menus, notices and surrogate pairs
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
faster loop for anything about *drawing* rather than about input. Since W10 it draws **either
palette**, which is the only way to look at the dark one without a Windows machine set to dark:

```sh
env -u DISPLAY wine target/x86_64-pc-windows-msvc/debug/grind-win32.exe book.fods \
    --render-to /tmp/frame.bmp
env -u DISPLAY wine target/x86_64-pc-windows-msvc/debug/grind-win32.exe book.fods \
    --render-to /tmp/dark.bmp --dark
```

A *window* still reads the registry, so seeing the running window dark — and seeing what this
shell does with somebody's **accent colour**, which no render can show, since a render is always
the suite's blue — takes a throwaway Wine prefix with the two values set. W10 used exactly this,
and it is the way to do it without touching your own prefix:

```sh
export WINEPREFIX=/tmp/wine-dark && wineboot -i
wine reg add 'HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize' \
     /v AppsUseLightTheme /t REG_DWORD /d 0 /f
wine reg add 'HKCU\Software\Microsoft\Windows\DWM' \
     /v AccentColor /t REG_DWORD /d 4283272960 /f   # 0xAABBGGRR — this one is a green
DISPLAY=:99 wine target/x86_64-pc-windows-msvc/debug/grind-win32.exe book.fods
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
  fallback. Writing it by hand — the recipe above — does exercise the dark palette and the accent
  path here, and both W3 and W10 used it: W3 to check the one thing this shell does *not* paint
  itself (the child `EDIT`s answer `WM_CTLCOLOREDIT` and come back dark with light text, where a
  control left alone is a white field in a dark window), W10 to see a green system accent reach
  the selection edge, the header bar and the assist band. What it cannot say is how the *system*
  chrome round all that looks: recent Wine has a dark mode of its own for common controls, so a
  themed listbox there is not evidence that a listbox on Windows would be themed, and the
  popup menus and scroll bars this shell leaves to Windows may look different there than here.
- Consolas is absent, so every screenshot exercises the `FIXED_PITCH | FF_MODERN` substitution
  path rather than the intended font.
- The IME path, and clipboard interop with real Excel. `IFileDialog`'s COM path *does* run
  under Wine — W3 opened a document through it — but it is Wine's own dialog, and how it looks
  and behaves on Windows 11 is unverified.
- The `*` dirty marker in the title, which Wine under a bare Xvfb draws no caption for.
- Per-monitor DPI, `WM_DPICHANGED`, and how any of it looks under Windows 11's compositor.

**In CI, two jobs — and they used to be three.**

1. `win32.yml`'s `check-from-linux` (ubuntu) — `cargo check --target`, `cargo clippy --target`,
   `cargo doc --target`, and `cargo test -p grind-win32` for the portable half. This is also the
   only clippy run this crate gets: `x86_64-pc-windows-msvc` is the Windows runner's own host
   target, so a second lint pass there read the same source through the same `cfg`s.
2. `artifacts.yml`'s `windows` (windows-latest) — **one release build**, and then everything
   that has something to say about the executable it produced:

   * `cargo test -p grind-win32` for the `cfg(windows)` half — the virtual-key codes checked
     against the real `winuser.h` values.
   * **Read the import table back** and fail on `vcruntime140*.dll`, `msvcp140.dll`,
     `ucrtbase.dll`, `api-ms-win-crt-*`, `mscoree.dll`, `hostfxr.dll`,
     `microsoft.windowsappruntime*` or `microsoft.ui.xaml*`.
   * The PE32+ stack reserve, and the version resource and icon (W8).
   * `--render-to` on two fixtures — a spreadsheet and a text document — **twice each, compared
     byte for byte**, with the size and the `BM` magic asserted so that two *empty* frames
     cannot agree their way past it. This is what makes a drawing refactor provable, and it is
     the one place the binary *is* run on a runner.
   * The size record and the `cargo bloat` breakdown, last, because `cargo bloat` rebuilds the
     binary with debug info and leaves it there.

   Except for the renders, the binary is never *run*: it is a GUI-subsystem application whose
   argument errors go into a message box, and a message box on a headless runner waits forever.
   The render path is safe because it opens no window and no message box on success — and **on
   failure it no longer opens one either**, which took two timed-out jobs to get right and is
   worth writing down:

   > `Start-Process -ArgumentList @(…)` joins its array with **spaces and quotes nothing**, so
   > the fixture `Sales Dashboard.fods` arrived at the exe as two arguments. `args::parse` said
   > "one file at a time", correctly — and `main.rs` put that sentence in a `MessageBoxW`, on a
   > runner whose session nobody is logged into. Nothing can click OK there, so the job printed
   > *nothing at all* and died on its 20-minute limit. The failure looked like a hang in the
   > drawing code and was an argument-quoting bug three layers away.
   >
   > Both halves are fixed, and each would have been enough on its own. `.github/scripts/win32-render.ps1`
   > runs the binary through `ProcessStartInfo.ArgumentList`, which applies Windows' real quoting
   > rules per element, captures both output streams, and bounds the wait at two minutes. And
   > `main.rs` now decides from the *raw* command line whether anybody is watching: `--render-to`
   > means nobody is, so every message — including the argument errors decided before the flag is
   > parsed — goes to stderr through `console_line` and never to a dialog. A GUI-subsystem process
   > has no console of its own but does inherit its parent's handles, so that reaches the runner's
   > log and is a silent no-op when launched from Explorer, where the exit code is the report.

   The frame is uploaded as an artifact, which is how the committed fixture the plan asked for
   gets produced: a `.bmp` from Wine cannot stand in for one from Windows, since every glyph
   there comes from a substituted font.

**Why two jobs and not the original three.** The `build` and `render` jobs were separate
`windows-latest` runners, and each compiled `grind-win32` in release to do its own half — plus a
third in the former `size.yml`, which compiled it again to weigh it. Three release builds of one
executable, on three runners, none able to see the others' output. They are now steps of one
job in `artifacts.yml`, which builds it once; the checks, the renders, the size and the
breakdown all read that. Nothing was dropped in the merge except the duplicate `cargo clippy`
noted above, which `check-from-linux` already ran for the same target. What could *not* be
merged is the Linux job: `cargo check` needs no MSVC and answers in about a minute, which is the
whole reason it exists.

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
