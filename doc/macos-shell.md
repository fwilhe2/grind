<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The macOS shell

The plan for `ui_mac/`: crate `grind-mac`, binary `grind-mac`, shipped as **`Grind.app`**.
It also records the decisions behind the plan. It is normative for that directory the way
`doc/windows-shell.md` is for `ui_win32/` and `doc/tui-shell.md` is for `ui_tui/`.
`doc/plan.md` reserved the directory name long before anything was in it, and ruled out the Mac
App Store (GPLv3 §6).

**M0 is this document**, plus a crate that parses its command line, a workflow that checks the
AppKit half from Linux, and a probe that measures on a real Mac what the rest of the plan would
otherwise have to assume. Nothing here opens a window yet.

## In one line

**An AppKit application over `grind-core`, holding both document types. It is written in Rust,
developed on a machine that is not a Mac, and built, run and looked at on GitHub's macOS
runners.**

## The problem this plan is shaped by

Nobody working on this repository has easy access to a Mac. So the question this document
answers first is not "how should a Mac shell look" but **how much of a Mac shell can be built and
tested without one**. The Windows shell asked the same question and answered it with a portable
half, `cargo check --target`, and Wine. Two of those carry over. Wine does not: there is no
equivalent for AppKit.

| | This Linux machine | A GitHub `macos-26` / `macos-15` runner | A person at a Mac screen |
|---|---|---|---|
| Type-check, lint and document the AppKit source | **yes.** `cargo check`, `clippy` and `rustdoc` never link, and objc2's framework crates are pure Rust with their bindings checked in, so no SDK is needed (*Evidence*) | yes | — |
| Run the portable half's tests: arguments, menus as data, selector tables, edit modes, geometry, notices, the Info.plist, the drive-script parser | **yes**, `cargo test -p grind-mac` | yes | — |
| Link a binary | **no.** Linking needs the macOS SDK, and the SDK's licence permits it on Apple hardware only, so osxcross and the like are out | **yes**, and it is the only place anything links | — |
| Run it | **no.** Darling runs command-line Mach-O programs, not AppKit; a macOS VM on non-Apple hardware breaks the macOS licence | **yes.** A logged-in GUI session, real system fonts, a real pasteboard, real LaunchServices | — |
| `--render-to` frames | no | **yes**, in real system fonts. A frame from the runner is *evidence* of what a user sees, where a frame from Wine was only a hint | — |
| A real window, driven | no | **yes.** Synthesized `NSEvent`s go through `NSApp.sendEvent` in-process, so no TCC permission is needed. Assertions are made on the *saved document*, and the snapshots are artifacts | — |
| Gatekeeper, LaunchServices "Open With", Quick Look, pasteboard interop | no | **yes**, through `spctl`, LaunchServices, `qlmanage` and `pbpaste` | — |
| Trackpad momentum, VoiceOver's speech, a real input method, how Liquid Glass looks at Retina | no | partly | **yes.** This is M12, over a screen-sharing session into a runner |

So the arrangement is `ui_win32`'s with the emulator replaced by the real thing on a runner.
**The portable half is as large as it can be made, the AppKit half is type-checked here on every
change, and everything that needs a Mac runs on one in CI.** From M11 that also includes an
ad-hoc scenario written on Linux: a drive script sent to a runner comes back as a transcript and
a set of PNGs without compiling anything. What cannot be decided by a machine is collected in one
list (*What the runner cannot speak for*) and settled in one sitting over screen sharing.

In two places the Mac is *better* placed than Windows was. CoreText is a shaping engine, so
decision 4 has none of GDI's gaps. And the runner is a real Mac, so its frames and its
interop checks are facts where Wine's were approximations.

## What the core already gives

The Win32 shell's table, unchanged. **This shell needs no new core API**, only the hoists
below, which move answers that already exist into the core rather than adding any.

| Need | Reached by |
|---|---|
| Which document type some bytes are | `grind_core::kind` |
| A rectangle of cells, with texts, styles and overlays | `grind_sheet::App::get_viewport` / `get_viewport_with` |
| Column widths and row heights | `App::col_widths` / `row_heights`, parsed with `grind_sheet::style::length_mm` |
| Typing a value or a formula, one undo step | `App::enter`, `enter_range`, `clear_range`, `preview` |
| Line breaking and every line-defined caret motion | `grind_core::layout` through `grind_text::App::layout_block` / `caret_x` / `caret_line` / `caret_line_bounds`, given a `Metrics` and a `Faces` |
| Direct character formatting; `**bold**` as it is typed | `App::char_style` / `set_char_style`; `App::type_markdown` |
| Opening and saving **from bytes** | `App::open_bytes` / `save_bytes`, which is exactly what `NSDocument`'s `readFromData:` and `dataOfType:` ask for (rule 5 paying off on a platform whose document architecture is data-first) |
| Typing help, friendly formulas, find | `grind_sheet::formula::assist`, `friendly`, `find::step` |
| The projection, the linter, the view modes | `App::project`, `App::lint`, `view::Overlays` |
| Repaint on change | `grind_core::Observer` — the core pushes, shells never poll |

## The decisions

### 1. One application, both document types

`Grind.app` opens a spreadsheet or a text document, chosen by `grind_core::kind` reading the
file's bytes. Bundle identifier `io.github.fwilhe2.Grind`, beside the GTK apps'
`io.github.fwilhe2.Sheet` and `.Text`.

The GTK shells are two applications **because a `.desktop` file's `MimeType=` is per
application**. A bundle's `CFBundleDocumentTypes` is a list, each entry with its own role,
icon and rank, so the reason to split does not exist here, exactly as it does not on Windows.
The display name is **Grind**: the menu bar's application menu, the Dock, About and the Finder
all say so.

### 2. AppKit, through `objc2`, from Rust

`objc2`, `objc2-foundation`, `objc2-app-kit`, `objc2-core-foundation`, `objc2-core-graphics`
and `objc2-core-text` are target-gated on `cfg(target_os = "macos")` with default features off
and only the classes used switched on (the *Evidence* shows why the second half matters). There
is no Swift, no Xcode project and no nib or storyboard. Windows, menus and toolbars are built in
code from tables, which is what lets the tables be tested on Linux.

Rejected:

- **Swift + UniFFI.** The shell could not be type-checked from Linux in any form, it needs a
  second toolchain on the one machine that has it, and `fwilhe2/editor`'s
  `doc/decision-win32-shell.md` records what the generated-bindings pin cost there.
- **cacao, egui, iced, Tauri.** A toolkit that draws its own chrome is not the platform's
  conventions, and the user asked for the platform's conventions.
- **An `NSTextView` holding the document** — rejected on architecture rather than effort. It
  owns a text storage, and the moment one exists there are two models. An `NSTextField` holding
  the formula bar's *input* is fine, for the reason `ui_win32`'s child `EDIT` is: it holds a
  keystroke, not the document, and committing goes through `App::enter`. The read-only
  `NSTextView` of decision 3's source view is also fine: it shows `App::project`'s output and
  is thrown away on every change.

### 3. Native chrome, drawn document — where this shell departs from Windows

`ui_win32` draws everything, because Win32's own controls needed a manifest the binary
deliberately lacks and Fluent's needed a runtime Windows does not ship. **Neither reason exists
on a Mac.** AppKit's controls are part of the system, they *are* the convention, and they take
on the system's current look for free. Liquid Glass on macOS 26 is the obvious case: an
`NSToolbar` built today is a Liquid Glass toolbar there and an ordinary one on 15, with no code
here knowing the difference. So the rule is:

**AppKit draws the chrome; this shell draws only the document.** The grid and the page are
custom `NSView`s (flipped, layer-backed). Everything around them is an AppKit control on one of
these surfaces, each with a membership rule:

| Surface | What may go on it | Instead of |
|---|---|---|
| **The menu bar** | every verb. It is the surface that grows, as on Windows (`doc/windows-shell.md` decision 4). **Help ▸ Search finds any menu item**, so the menu bar is also this platform's command palette, for nothing | `ui_sheet_gtk`'s ⌘K palette |
| **The toolbar** (`NSToolbar`) | a *property of the selection* only, which is the format bar's admission test, bounded by `CellStyle`, `numfmt::Format` and `CharStyle`. Every item also has a menu twin, because a Mac user may hide the toolbar, and a test holds that | the drawn format strip |
| **The sidebar** (`NSSplitViewController`, a source list) | *where you can go*: sheets and defined names, or headings and bookmarks, plus a **Problems** section (D6) whose every row is a jump | Win32's go-to, outline and lint dialogs |
| **The titlebar accessory** (`NSTitlebarAccessoryViewController`, bottom) | the name box and the formula field, which are the two read-outs of *where* and *what* | Win32's strip |
| **Context menus** | built from the menu table, so a row and its menu-bar twin cannot disagree | — |
| **The source pane** | D9's projection, as a trailing split item with a read-only `NSTextView`, toggled by View ▸ Show Source (⌥⌘U, Safari's key for the same idea) | Win32's modal listbox |

A notice (a recalculation refused, a parse error) is a banner under the accessory. It is still
a *state*, never an event, and `notice.rs` holds every sentence, as in `ui_win32`.

### 4. CoreText measures and draws — decision 3 of the Windows shell, answered by a shaping engine

`doc/windows-shell.md` decision 3 is the argument this one rests on: **the engine that measures
and the engine that draws must be the same one**, or the caret and the ink disagree. GDI met
that rule by drawing with the advances it had measured, and paid for it in every script that
needs shaping.

CoreText meets the same rule with none of that cost:

- `Metrics::advances` comes from one `CTLine` per fragment: the caret offset at the UTF-16
  index after each character, folded to one advance per `char`. The fold is `ui_win32`'s own
  `metrics.rs` logic, since `NSString`, like `WCHAR`, counts UTF-16 units. The offsets are
  cluster-aware, so a decomposed `é`, a ligature, an emoji ZWJ sequence and a Devanagari
  conjunct are each one caret stop and one box. The probe measures exactly this.
- Each fragment is drawn with **the same `CTLine`** at the x the core measured. Font fallback
  happens inside CoreText on both halves, so a glyph drawn from a fallback font is also
  measured in it.
- RTL remains out (`doc/text-layout.md`). That is a decision of the suite's, not a gap in this
  shell.

**Lengths are 72 points to the inch**, which is how every Mac document application measures a
page. A 2.5cm column is 70.9pt wide here, where Windows' 96-dpi convention makes it 94.5px. The
document means centimetres in both cases; each platform decides what a centimetre is on its
screen.

### 5. `NSDocument`, one window per document, **autosave in place** — *decided with the user*

A subclass of `NSDocument` (`objc2`'s `define_class!`) holds an `App`, and AppKit's document
architecture supplies most of what "a Mac document application" means with no code here.
**That includes things this suite has never had:**

- Open Recent, the Edited dot in the close button, rename and move from the title bar's proxy
  icon, Duplicate, Revert To and the Versions browser, window tabs (Window ▸ Merge All Windows),
  window restoration on relaunch, and File ▸ Share.
- `readFromData:ofType:` is `App::open_bytes` and `dataOfType:` is `App::save_bytes`, so R6's
  splicing is what every save writes, autosaves included. **An autosave is one line of diff**,
  the same as a ⌘S.

**Autosave in place is on.** It has been the Mac convention since 10.7 — TextEdit, Preview and
Pages all do it — and the user chose the convention. This is the first client in the suite with
autosave. `doc/feature-matrix.md` §9 lists autosave as absent from every client, and
`doc/sheet-shell.md` calls it "a named later toggle". Both are amended in M4, when it is built,
with the line moved to "the Mac only, by platform convention". What makes it safe here rather
than reckless:

- **An untouched document is never written.** `NSDocument` autosaves only a document with
  changes, and the observer is registered after the load, so opening a file is not an edit.
  M4's exit criterion asserts the bytes *and* the modification time.
- **An imported workbook or CSV is untitled.** It opens as a new document under
  `grind_xlsx::suggested_name` with no file URL, so there is nothing to autosave *into*, and an
  untitled autosave goes to `~/Library/Autosave Information`, never beside the source. One way in,
  never out (`doc/not-doing.md` §1), is kept by construction rather than by care.
- **Undo is still the core's** (rule 2). `hasUndoManager` is NO, and Edit ▸ Undo and Redo are
  `undo:` and `redo:` actions this shell answers by forwarding to `App`. `validateMenuItem:` greys
  them when there is nothing to undo, which is `ui_win32`'s named gap closed by the platform's
  own mechanism. The core's observer calls `updateChangeCount:`, which is what the Edited dot and
  the autosave timer read.

**One document per window.** The Windows shell swaps a window's `Pane` when File ▸ Open is used;
**that does not carry over**. On a Mac, Open makes a new window (or a tab, if the user has asked
for tabs), and a document-based application with no windows is still running. The shell's own
state is per document rather than per window, which is the shape `Pane` already was.

### 6. A welcome window, and a recent list for nothing — *W11 in the Mac's shape*

`ui_win32` decision 11 applies unchanged in substance: a launch with nothing to open shows the
*choice* — New Spreadsheet, New Text Document, Open… — rather than guessing. On a Mac that is
`applicationShouldOpenUntitledFile:` answering NO and showing a small window of the shell's own,
which also comes back on a Dock click with no windows open (`applicationShouldHandleReopen:`).
Each choice runs a menu command, W11's rule.

It is designed here and not borrowed: not Numbers' or Pages' template gallery, which is a
different idea (templates are gated with the rest in `doc/not-doing.md`). **The recent-documents
list W11 declined to build comes free**, because `NSDocumentController` keeps it. It is the
system's list rather than a file of ours, which is exactly the shape W11 said the answer should
take.

File ▸ New is a pair here as well: New Spreadsheet ⌘N and New Text Document ⌥⌘N.

### 7. Keys are **selectors**, not key codes

A custom `NSView` gets a key as `keyDown:` and hands it to `interpretKeyEvents:`. The system
then answers with **what the key means**: `insertText:` for text, or `doCommandBySelector:`
with `moveWordLeft:`, `moveToBeginningOfDocument:`, `deleteBackward:` and the rest of
`NSStandardKeyBindingResponding`. That table is the user's: it honours
`~/Library/KeyBindings/DefaultKeyBinding.dict`, the Emacs keys every Mac text field answers
(⌃A, ⌃E, ⌃K), and the platform's own idea of what ⌥← and ⌘↑ do.

So `keys.rs` is a portable map **from selector name to action**, for the grid and the page
alike. It is `ui_win32`'s virtual-key table in the Mac's vocabulary, and it is testable on Linux.
Its platform pin is the same move `ui_win32` made against `winuser.h`: a runner-only test reads
the system's own `StandardKeyBinding.dict` and asserts that every selector bound there is either
answered or named as deliberately unanswered.

Two consequences:

- **The page implements `NSTextInputClient` in full**, marked text included and drawn inline in
  the page's own ink. Dead keys (⌥E then E), press-and-hold accents, the emoji picker
  (⌃⌘Space) and every input method go through it. That puts this shell ahead of `ui_win32`,
  whose IME answer is the system's floating box.
- **On the grid, a printable key opens the in-cell editor and hands the same event to its field
  editor**, so a dead key or an input method can *start* an edit. It is the Mac form of
  `ui_win32`'s split between `on_key` and `typed`.

### 8. Colour: the system's semantic colours, and the core's rule for the document's own

Chrome colours are `NSColor`'s semantic ones (`textBackgroundColor`, `labelColor`,
`separatorColor`, `selectedContentBackgroundColor`, `controlAccentColor`), resolved in the view's
`effectiveAppearance` at draw time. So dark mode, high contrast and the user's accent follow the
system with no theme table of ours. A colour **the document** chose goes through
`grind_core::color`'s automatic ink and document ink, the rule every other shell applies. That
is what keeps a navy word readable on a dark page, and a grey heading row readable in either
appearance. `--render-to` pins the appearance (`--dark`) and the accent (the system default), so
a frame is a function of its command line alone.

### 9. `--render-to` and `--drive` — assertable output, not user features

- **`--render-to <file.png> [--dark]`** draws one frame into a `CGBitmapContext` with no
  application object and no window server, through the same `draw_frame` the views' `drawRect:`
  calls. This is `ui_win32` decision 5 on CoreGraphics. Two renders are asserted byte-identical
  on the runner, with the size and the PNG signature checked so that two empty frames cannot
  agree their way past it. Because only the document is drawn by this shell (decision 3), a
  render is the grid or the page, not the window around it.
- **`--drive <script> --out <dir>`** opens a real window and replays a script of lines like
  `key cmd+b`, `type Total`, `click 120,40`, `menu Format/Bold`, `snap bold` and `save`. The events
  are synthesized `NSEvent`s handed to `NSApp.sendEvent`, so they take the path a real key
  takes: key equivalents and the menu bar, the responder chain, `interpretKeyEvents:`. This
  happens in-process, so it needs no Accessibility or Input Monitoring permission. `snap` caches
  the window's frame view into a bitmap, which needs no Screen Recording permission either. A
  transcript (selection, caret, notice, dirty state) goes to stdout. **The assertions are made on
  the saved document** through the CLI (`grind sheet view`, `grind lint`, `grind sheet project`),
  which is what makes a drive a test rather than a picture. The PNGs are for a person to look at.

The script parser is portable (`drive.rs`) and tested on Linux. Only the replay needs a Mac.

### 10. Accessibility: a floor, then `accesskit_macos`

The floor is what `ui_sheet_gtk`'s M9 and `ui_win32`'s system caret are. The page view speaks
`NSAccessibility`'s text-area protocol: its value, the selected range and the insertion line. A
move on the grid posts an announcement. Both are read back in-process on the runner by a
drive's `a11y` step (M9 — planned as `tests/appkit.rs`, which a binary crate cannot build), so the
floor is asserted rather than hoped for. The named upgrade is
`accesskit_macos` (objc2-based and maintained, at 0.27 as this is written), which is the answer
the day a full element tree for the grid matters.

### 11. macOS 15 and later, universal

The deployment target is **macOS 15**, set in `.cargo/config.toml`, because that is the oldest
macOS GitHub still runs (macOS 14 leaves the runners in November 2026). **A floor that is never
run is a claim that is never checked.** The release is universal2: both slices are built on one
arm64 runner and joined with `lipo`. macOS 26 is the last release for Intel Macs, and the x86_64
slice stays as long as it costs nothing.

### 12. Signed ad hoc, for now — *decided with the user*

Apple silicon runs no unsigned code, and the linker's ad-hoc signature satisfies that for a
binary built on the machine that runs it. A **downloaded** app is quarantined, and since macOS
15 an ad-hoc-signed one opens only after System Settings ▸ Privacy & Security ▸ Open Anyway. A
right-click no longer bypasses Gatekeeper. And Homebrew's main cask repository has refused
un-notarized software since 2026-09-01.

So M10 ships an ad-hoc-signed DMG from the runner, plus a Homebrew tap of the project's own with
the Open Anyway step written down. **Developer ID and notarization are a later decision that
costs no code**: a certificate and a notary key as CI secrets, and two more steps in the same job.
The day they are added, the `spctl` assertion in M10 flips from "rejected, as expected" to
"accepted", and the test says so.

## The crate

`*` compiles and runs its tests on any host. `[M]` needs macOS. `[~]` is portable with an `[M]`
half beside it.

```
ui_mac/
  Cargo.toml            * grind-mac; the objc2 family target-gated, default features off
  examples/probe.rs    [~]* M0's measurements (below); retired when --render-to and --drive exist
  src/
    main.rs             *   argv, the kind sniff, and — until M2 — what a window *would* open
    args.rs             *   the command line, including AppKit's own `-Key value` pairs (M0)
    import.rs           *   which bytes are imported rather than opened (M0: the sniff only)
    menu.rs             *   the menu bar as data: title, key equivalent, and an action that is
                            either a standard selector or a `Command` (M2)
    keys.rs             *   selector name → grid action / page action (M3, M6)
    notice.rs           *   every sentence the banner says, with keys spelled ⌘ (M4)
    welcome.rs          *   the three choices, the layout of the window and its frame (M9)
    welcome_window.rs  [M]  the welcome window's view (M9)
    a11y.rs             *   what the accessibility floor says (M9)
    places.rs code.rs   *   the sidebar's rows and the source pane's lines (M8)
    tools.rs            *   the toolbar as data (M7)
    plist.rs            *   Info.plist, generated from the same table of types the tests read (M10)
    drive.rs           [~]  the drive-script parser, and its replay (M2)
    ops.rs              *   a frame as a list of things to draw, for either pane (M2, M6)
    sheet/{geom,paint,select,state,search}.rs  *   the portable half of the grid, over the hoists
    text/{geom,face,state,input,paint}.rs      *   the portable half of the page (M6)
    app.rs             [M]  the application delegate and the menu bar built from `menu.rs`
    document.rs        [~]  the type names and the byte sniff; the NSDocument subclass holding an `App`
    grid_view.rs       [M]  the grid, inside an NSScrollView
    page_view.rs       [M]  the page, and NSTextInputClient (M6)
    watch.rs           [M]  the core's observer, bridged to either pane on the main thread
    accessory.rs banner.rs editor.rs find_bar.rs sidebar.rs clipboard.rs  [M]
    toolbar.rs         [M]  the format bar (M7)
    metrics.rs         [~]  CoreText behind `Metrics`; the UTF-16 fold is portable
    render.rs          [M]  the `Op`s onto a CGContext, and the bitmap `--render-to` draws into
    png.rs              *   the frame's PNG, written here so two renders are the same bytes
  (tests/appkit.rs was planned here; a drive's `a11y` step is what M9 built instead)
  data/                     grind.svg → Grind.icns by make-icns.py (M10); Info.plist is generated
```

**AppKit must be called from the main thread**, and the Rust test harness runs tests on worker
threads. The `[M]` assertions that need an application object are therefore made by drives, which
run the real application on its main thread (*What M9 built* says why not a `harness = false`
test); the `[M]` unit tests are the ones that need no application — CoreText's, in `metrics.rs`.

## M1 — hoist before copying

The ledger says when a copy must stop being a copy, and a Mac shell pulls several of those
triggers at once. **This is the milestone before the shell, not a cleanup after it.**

| What | Copies today | The ledger's trigger | Destination |
|---|---|---|---|
| Sheet navigation: `Selection`, `Motion`, `moved`, the Ctrl+arrow data-edge rule, `onto_visible` | `ui_win32/src/sheet/keymap.rs`, `ui_sheet_gtk/src/keymap.rs`, `ui_tui/src/sheet/keymap.rs`, `ui_web/src/sheet/keymap.rs` | `ui_win32/src/sheet/keymap.rs`'s `ponytail:`: "the trigger is a **third** copy" | `grind_sheet::nav`, adopted by `ui_win32` and `ui_sheet_gtk`, whose vocabularies were kept identical for this |
| The text pane's `Flow`, including table placement | `ui_win32/src/text/geom.rs`, `ui_text_gtk/src/geom.rs` | `ui_win32/src/text/geom.rs`'s `ponytail:`: "the trigger is a third copy" | `grind_text::flow`, adopted by both |
| The status bar's Sum / Count / Average over `App::preview` | `ui_sheet_gtk/src/chrome.rs`, `ui_win32/src/sheet/status.rs`, `ui_tui/src/sheet/app.rs` | `ui_tui`'s `ponytail:`: "a fourth caller is where it gets hoisted" | `grind_sheet`, adopted by all three |
| The TSV clipboard codec | `ui_win32/src/sheet/clip.rs`, `ui_sheet_gtk/src/grid.rs`, `ui_web/src/sheet/mod.rs`, `ui_tui/src/sheet/app.rs` | four copies already | `grind_sheet::clip` |
| Track prefix sums (`Sizes`), with the unit per millimetre as a parameter | `ui_sheet_gtk/src/geom.rs`, `ui_win32/src/sheet/geom.rs` | the Mac would be the third | `grind_sheet::tracks` |
| UTF-8 ⇄ UTF-16 offsets | `ui_win32/src/sheet/state.rs` | `NSRange` counts UTF-16 units, as `EM_GETSEL` does | `grind_core` |
| The file-decides-its-kind rule (`reconcile`, `describe`) | `ui_tui/src/main.rs`, `ui_win32/src/args.rs`, and now `ui_mac/src/args.rs` | this crate's own `ponytail:`, written in M0 | `grind_core::kind` |
| The edit modes (Ready / Enter / Edit) and a cell format toggle's read-change-write | `ui_sheet_gtk/src/state.rs`, `ui_win32/src/sheet/state.rs`; `ui_win32/src/sheet/format.rs` and the GNOME format bar | the Mac would be the third | hoisted if one abstract key type fits all three shells' inputs, and otherwise mirrored with a `ponytail:` naming the fourth |

Each hoist is its own commit, and **each is proved a refactor rather than argued one**:
`--render-to` from `ui_win32` (under Wine, `env -u DISPLAY`) and from both GTK windows,
byte-identical before and after. The precedent is `formula::assist` and `grind_text::format::Change`.

### What M1 did

**Done.** Every row above is answered, one commit each, and two rows grew on the way because the
Mac would have copied something the table had not listed:

| Row | Became | What the hoist found |
|---|---|---|
| Navigation | `grind_sheet::nav` | The GTK step added without saturating and the Windows one saturated; the hoist saturates |
| Tracks | `grind_sheet::tracks` | The union of both copies' methods; the millimetre conversion is a function, so `ui_win32` keeps its own order of arithmetic |
| Aggregates | `grind_sheet::summary` | The terminal keeps its own spelling over the same numbers |
| Clipboard | `grind_sheet::clip`, the line ending a parameter | The browser and the terminal pasted a formula back as `#NAME?`: neither turned display syntax into ODF's |
| `Flow` | `grind_text::flow`, with each shell's numbers as a `Spacing` and cell measures from one `across` map | Three drifts: GTK left two gaps above a table, only Windows indented a list item inside a cell, only GTK gave `Title`/`Subtitle` a heading's space. The GTK frame changed by exactly the first, proved against the old code with that one line changed |
| UTF-16 | `grind_core::utf16` | Windows' copy put the caret at the end of a formula for a byte inside a character; the browser counted `char`s where the DOM counts UTF-16 units, and could panic on a parse error's offset. It is the third caller |
| The kind rule | `grind_core::kind::reconcile`, and `DocumentKind::label` for `describe` | `describe` was a copy of `label` that called a presentation a "document" |
| Edit modes | **Mirrored**, with a `ponytail:` on both machines naming the fourth copy | No one abstract key type fits: GTK's `Key` carries its character, Windows takes `WM_KEYDOWN` and `WM_CHAR` apart and asks its accelerators first, and the Mac gets selectors from its field editor. The machines also differ on purpose — pointing and F4 exist only in GTK |
| — what a commit stores | `grind_sheet::formula::display::to_input` | Five copies of the same four-line match, one of them `App::replace`'s |
| Format toggles | `grind_sheet::format` — `Toggle`, `restyled`, `coloured`, `stepped` | The browser left an empty `style:style` behind, read `oblique` as not italic, and stepped a date's decimals into a number format |

So the Mac's `sheet/state.rs` (M4) is the one copy it writes on purpose, and it starts from
`to_input`, `nav` and `format` rather than from either shell.

## Milestones

Every milestone lands green: `cargo fmt --check`, clippy for the host **and both Apple targets**,
`cargo doc`, `reuse lint`, the portable tests, and, from M2, the runner's job.

| # | Milestone | Exit criterion |
|---|---|---|
| **M0** | **Plan and wiring** — this document; `ui_mac/` with `args.rs` and `main.rs`; a workspace member (not a default member); `macos.yml`; `-p grind-mac` in `ci.yml`'s host lists; `ui_mac` in `cli/tests/packaging.rs`'s `UNPACKAGED`; the deployment target | The Linux half is **met** (see *Evidence*). The runner half is **the first run of `macos.yml`'s `probe` job**: the core works on macOS, CoreText draws headless and reproducibly, a window opens and can be snapshotted on both OS versions, and synthesized events reach a text field. Each answer goes into *Evidence*, and a "no" rewrites the decision that depended on it before M2 starts |
| **M1** | The hoists above — *done* (*What M1 did*) | All suites green; Win32 and GTK frames byte-identical before and after each — **met**, with the one GTK frame change the `Flow` hoist made on purpose, proved separately |
| **M2** | **The application, the document and the read-only grid**: the menu bar from `menu.rs` (application, File, Edit, Format, View, Window, Help — standard selectors); the `NSDocument` reading all three forms, with workbooks and CSV as untitled; the grid in an `NSScrollView` (elastic, overlay scrollers, headers as floating subviews); the document's own widths and heights; hidden tracks gone; CoreText cells with `numfmt::overflow`'s hashes; both appearances; `--render-to`; `--drive`; the `macos` job in `artifacts.yml`, which retires the probe — *built, and not yet run* (*What M2 built*) | Every R7 and sample document opens on the runner through a drive script, with a snapshot each; renders byte-identical ×2 in both appearances, on 15 and on 26. **The Linux half is met; the runner half is the first run of `artifacts.yml`'s `macos` job** (`.github/scripts/mac-frames.sh`) |
| **M3** | Selection and navigation from selectors over `grind_sheet::nav`; click, drag and the header bands; the name box; the status bar over the hoisted aggregates; sheets in the sidebar — *built, and not yet run* (*What M3 built*) | A drive's transcript ends on the expected selection for arrows, ⌘-arrows, Shift-extension and a typed `g20`. **The Linux half is met; the runner half is `mac-frames.sh`'s selection drive**, which checks the transcript after every step |
| **M4** | **Editing and saving**: the formula field and the in-cell editor, the three modes, display syntax in and ODF out, a parse error keeping the edit open, undo and redo validated, **autosave in place**, Versions, Revert and Duplicate, sheet add/rename/delete, the notice banner. `doc/feature-matrix.md` §9, `doc/sheet-shell.md` and `doc/not-doing.md` amended for autosave — *built, and not yet run* (*What M4 built*) | A driven edit autosaves a file that lints clean and projects as expected; **an untouched open writes nothing**, bytes and modification time; an imported workbook's source is never touched. **The Linux half is met; the runner half is `mac-frames.sh`'s editing section** |
| **M5** | The pasteboard (plain text and TSV) and Edit ▸ Find (⌘F, ⌘G, ⇧⌘G, ⌘E) over `App::find` / `replace` / `find::step` — *built, and not yet run* (*What M5 built*) | Copy in the app and `pbpaste` in the job shows the TSV; `pbcopy` in the job and paste in the app lands in cells. This is cross-application interop, which `ui_win32` could not verify under Wine. **The Linux half is met; the runner half is `mac-frames.sh`'s pasteboard section** |
| **M6** | **The page**: CoreText `Metrics` and `Faces`, `grind_text::flow`, `NSTextInputClient` with inline marked text, selection, `type_markdown`, the selector actions, tables drawn as a grid — *built, and not yet run* (*What M6 built*) | The page breaks where `grind text view --width` does (`Fixed`, as in W5a); CoreText tests show NFD at precomposed width, and a ZWJ family and a Devanagari conjunct each as one caret stop; a drive types `**bold**` and composes `é` through `setMarkedText`; renders ×2. **The Linux half is met, and the probe measured the CoreText half (*Evidence*); the runner half is `metrics.rs`'s Mac-only tests and `mac-frames.sh`'s page section** |
| **M7** | **Formatting**: toolbar items (bold, italic, underline, strike and code as one segmented control; number format and paragraph style as pop-ups; `NSColorWell`s opening the system colour panel); Format ▸ Font through the system font panel (`changeFont:` becomes a `grind_text::format::Change`); the grid's cell formatting — *built, and not yet run* (*What M7 built*) | A drive makes a label bold, steps a currency's decimals, colours a cell and clears it; the saved file projects `style … bold=#true …`. **The Linux half is met; the runner half is `mac-frames.sh`'s formatting section** |
| **M8** | Formula literacy and the shared panes: completion and signature over `formula::assist`, the friendly formula bar, the function list, Explain; the source pane (D9); Problems in the sidebar (D6); the roles and names overlays (V7) — *built, and not yet run* (*What M8 built*) | A drive types `=SU` and Tab takes an offer; every Problems row jumps; with every overlay on, a save is byte-identical. **The Linux half is met; the runner half is `mac-frames.sh`'s eighth section** |
| **M9** | The welcome window and Open Recent; context menus; the accessibility floor — *built, and not yet run* (*What M9 built*) | ~~`tests/appkit.rs`'s~~ **a drive's `a11y` step's** accessibility assertions pass; the welcome window renders in both appearances. **The Linux half is met; the runner half is `mac-frames.sh`'s last two sections** |
| **M10** | **Packaging**: `plist.rs` writes the Info.plist (every type at `LSHandlerRank` **Alternate** — *offered, never taken*, `assoc.rs`'s stance in Mac terms — except `.grind`, which nothing else opens, at **Owner**, conforming to `public.plain-text` so Quick Look previews one for free); an icon on the macOS 26 grid; universal2; ad-hoc signed; a DMG; the tap — *built but for the tap, and not yet run* (*What M10 built*) | `plutil -lint`; `codesign --verify --strict`; `otool -L` lists only `/System/Library` and `/usr/lib`, the Mac's version of the Windows import-table check; `vtool` reports a minimum of 15.0; LaunchServices offers Grind for a `.fods` without becoming its default; `spctl` rejects the ad-hoc build, **as expected, and asserted as such**. **The Linux half is met; the runner half is `.github/scripts/mac-bundle.sh`** |
| **M11** | **The remote loop**: `scripts/mac-remote.sh renders`, `… drive <script>` and `… session`; `mac-drive.yml`; a Mac column in `doc/feature-matrix.md`; `CLAUDE.md` — *built, and not yet run* (*What M11 built*) | A drive script written on Linux comes back as a transcript and PNGs in minutes, compiling nothing. **Met by construction and unproved: the first dispatch of `mac-drive.yml` after a DMG exists is the proof** |
| **M12** | **The UX pass**, over a screen-sharing session into a runner (`mac-session.yml`), recorded the way `doc/windows-shell.md`'s is | Every item on *What the runner cannot speak for* is either checked or still named |

### What M2 built

Everything M2 names exists, and **none of the AppKit half has run**: it is written against
objc2's generated bindings, type-checked and linted for both Apple targets from Linux, and the
first thing that will run it is `artifacts.yml`'s `macos` job. Until then this is a claim about
what compiles, not about what works.

The split is decision 9's, taken as far as it goes. Every decision a user would notice is
portable and tested here; only turning it into pixels and events needs a Mac:

| File | Half | What it is |
|---|---|---|
| `sheet/geom.rs` | portable | Every cell in points, in the sheet's own coordinates: one-inch columns and quarter-inch rows by default (`ui_web`'s and `ui_tui`'s sizes), the document's own through `grind_sheet::tracks`, rows hidden by hand or by a filter closed up, and a grid that reaches the used extent and a margin rather than a million rows |
| `sheet/paint.rs` | portable | A frame as a list of `Op`s — a fill, or a line of text at a place — over `grind_sheet::look`, `numfmt::overflow` and `grind_core::color::document_ink`; the header bands, and a whole frame composed from them. Tested against `grind_core::layout::Fixed` |
| `metrics.rs` | both | The UTF-16 fold, portable and tested; CoreText behind `Metrics`, whose `CTLine` is also the one drawn |
| `png.rs` | portable | The frame's PNG, written here over the workspace's own `flate2`, so two renders are the same **bytes** — a system encoder may write a timestamp or a profile of its choosing |
| `menu.rs` | portable | The menu bar as data, and *Conventions made mechanical* as its tests |
| `drive.rs` | both | The drive-script parser, portable, and its replay through `NSApp.sendEvent` |
| `import.rs` | portable | A workbook or a CSV to flat ODF **in bytes**, the shape `NSDocument` hands a document over in |
| `document.rs` | both | The type names and the byte sniff, portable; the `NSDocument` subclass and the document controller, whose `typeForContentsOfURL:error:` reads the file rather than its name |
| `render.rs` `grid_view.rs` `app.rs` | Mac | The `Op`s onto a flipped `CGContext`; the grid in its scroll view with the bands floating; the application, its delegate and the menu bar |

What M2 leaves, each on purpose: the selection (M3), editing, saving and autosave (M4 — the
document writes, but nothing yet changes it), a label spilling into the empty cells beside it (a
cell's text is clipped to its own cell), borders, the welcome window (M9 — a launch with nothing
named starts an empty spreadsheet), a text document's page (M6 — its window says so), and
Info.plist document types (M10 — the document controller answers by the bytes, and the open panel
shows every file, so M2 needs none). **The probe's answers are in
*Evidence*** (run 36679496310), and it stays one step longer than planned: `artifacts.yml`'s
`macos` job has been red since its first run, and until it is green the probe is the only Mac job
that says anything works. `mac-frames.sh` now reports its own failure as an annotation, so the
next red run says why.

### What M3 built

The same arrangement as M2's, and the same caveat: built, type-checked for both Apple targets,
and not yet run. Two more hoists came first, because the Mac's name box and its keyboard would
otherwise have been a third copy each: `grind_sheet::place` (the name box's text, `locate`, and
the status bar saying nothing for one cell) and `nav::occupied` with `nav::all` (whose Select All
now keeps the active cell at A1 in every shell — the GNOME window had put it at the far corner).

| File | Half | What it is |
|---|---|---|
| `keys.rs` | portable | Decision 7's table: a selector to a grid action, with ⌘-arrows as the data's edge, each `…AndModifySelection:` its plain twin extending, and every binding left alone named with its reason in `UNANSWERED` — the list the runner's `StandardKeyBinding.dict` check will read |
| `sheet/select.rs` | portable | What an action, a click, a drag or a header click makes of the selection, over `nav`; how far a page goes; what to scroll into sight beside the bands |
| `sheet/paint.rs` | portable | The selection: a wash over the range but its active cell, an outline over everything, the selected tracks tinted in the bands |
| `grid_view.rs` | Mac | Keys through `interpretKeyEvents:` and `doCommandBySelector:`, clicks and drags, the bands' own clicks, and `Pane::select`, which redraws, reveals and tells the listeners |
| `accessory.rs` | Mac | The titlebar accessory: the name box, where typing a place and Return goes there, and the status read-out |
| `sidebar.rs` | Mac | An `NSSplitViewController` whose sidebar is a source list of the sheets; choosing one shows it |

Go To (⌘L) is a new `Command` — the keyboard into the name box — because a drive, like a person,
can only reach the box through a key.

### What M4 built

Built and type-checked, not yet run — the same caveat as M2 and M3. One more hoist came first:
`App::fresh_sheet_name`, since the Windows pane's Add Sheet had been suggesting a name the core
then refused.

| File | Half | What it is |
|---|---|---|
| `sheet/state.rs` | portable | The edit-mode machine, fed selectors — the third copy M1 decided to mirror, with its `ponytail:` naming a fourth as the trigger |
| `notice.rs` | portable | Every sentence the banner says, with ⌘ for keys and a test that none says Ctrl |
| `editor.rs` | Mac | One `NSTextField` over the active cell; its delegate's `control:textView:doCommandBySelector:` goes through `sheet/state.rs`; a commit is `display::to_input` and `App::enter`, and a formula that will not parse keeps the edit open with the caret on the problem |
| `banner.rs` | Mac | A second titlebar accessory, hidden with nothing to say; Recalculate Anyway is its one button |
| `grid_view.rs` | Mac | Typing starts an edit, a double-click or ⌃U amends the cell, `undo:` and `redo:` are the core's and validated by `can_undo`/`can_redo`, and the core's observer reaches the pane through a registry the main thread owns — the observer must be `Send + Sync`, and a pane holding views is not |
| `document.rs` | Mac | Autosave in place on, no undo manager, and every change the core reports marking the document edited — which is what keeps an untouched open from writing anything |
| `sidebar.rs` `menu.rs` | both | Insert ▸ Sheet, Format ▸ Rename Sheet…, Edit ▸ Delete Sheet, where Excel for Mac has them, with the sidebar's list following every change |

What M4 leaves: the formula field edits only through the cell editor (the read-out follows it —
a `ponytail:` in `accessory.rs`); an undo back to the saved state still leaves the Edited dot,
since the core's observer says *changed* rather than *which way*; and ⌃U inside the editor is the
field editor's, so toggling Enter and Edit mode there is not yet reachable.

### What M5 built

Built and type-checked, not yet run. One more hoist came first, and it fixed a bug: `nav::target`,
the rectangle an operation over a selection acts on, is the Windows pane's rule (only a whole
row or column is cut to what the sheet uses) — the GNOME window had cut every rectangle, and the
Windows pane's Copy had cut none, so copying a whole column there put a million lines on the
clipboard.

| File | Half | What it is |
|---|---|---|
| `sheet/search.rs` | portable | Where Find goes next: `App::find` and `find::step`, across sheets and wrapping |
| `notice.rs` | portable | Found, not found and replaced, in ⌘-spelled sentences |
| `menu.rs` | portable | Edit ▸ Find as the platform's own `performFindPanelAction:` with `NSFindPanelAction`'s tags — ⌘F, ⌘G, ⇧⌘G, ⌘E — tested for their keys like every other item |
| `clipboard.rs` | Mac | Copy, Cut, Paste and Delete; the one file that touches `NSPasteboard`, writing the TSV as plain text and as tab-separated text, and reading plain text back through `App::enter_range` |
| `find_bar.rs` | Mac | A third titlebar accessory: a search field, a replace field, Replace All and Done |

Also mended while here: Edit ▸ Select All and the Delete keys reach the grid, which M3 and M4
had named and not wired.

### What M6 built

Built and type-checked, not yet run — with one difference from M2–M5: the probe had by then
measured the CoreText half on both OS versions (*Evidence*), so decision 4 rests on a
measurement rather than on the documentation. Four hoists came first, each a copy the Mac
would otherwise have been the third or fourth of: `grind_text::caret` (a character, a word and a
click), `grind_text::look` (which face a block is set in), `grind_text::paint` (what a laid-out
line is drawn as — the pieces, the selection band, the bullet) and `App::layout_composing` (an
input method's composition, laid out as if typed).

| File | Half | What it is |
|---|---|---|
| `ops.rs` | portable | The one drawing vocabulary for both panes — `Op` left `sheet/paint.rs` and gained `Run`, a piece of a line in a resolved `Font`, with its underline and strike |
| `metrics.rs` | both | `Font`, the one currency between measuring and drawing; `Face`, the page's `Metrics` per role; CoreText's user fixed-pitch face and a family the document names; and Mac-only tests holding the shaping answers |
| `text/geom.rs` | portable | The page's numbers: a `Spacing`, a column centred at a readable measure, a bullet hung in its indent |
| `text/face.rs` | portable | A block's role and a run's own formatting resolved into one `Font`; `Column`, the page's `Faces`; `lay_out`, one layout for the view and for `--render-to` |
| `text/state.rs` | portable | The caret, the selection and the composition; every motion and edit the page makes, over a real `grind_text::App` |
| `text/input.rs` | portable | `NSTextInputClient`'s arithmetic: the caret's block as the string an input method sees, and every UTF-16 range converted once |
| `text/paint.rs` | portable | A frame of the page as `Op`s: runs in their fonts, the selection, the caret, a table's rules, a bullet, and marked text **inline and underlined** — ahead of the Windows pane, whose composition is the system's floating box |
| `keys.rs` | portable | The page's selector table beside the grid's, and `PAGE_UNANSWERED` with its reasons |
| `page_view.rs` | Mac | The page in its scroll view, the input client, clicks by the count, the standard Edit selectors, and the core's notification held back while the view's own edit runs |
| `watch.rs` | Mac | The observer registry `grid_view.rs` had, behind a trait, for both panes |
| `drive.rs` | both | `mark` and `commit`, the two calls an input method makes; a click lands in whichever view the document has |

What M6 leaves: the caret did not blink (since closed: `text/blink.rs`, steady while the user
types and never started under a drive, so a snapshot does not depend on when it was taken); a selection across
blocks is shown to an input method as the caret alone, since the one block it sees cannot hold
it; a picture outlined where it goes rather than drawn (since closed: `NSImage` decodes it); the text sidebar (headings and bookmarks),
paragraph kinds from a menu and every formatting control, which are M7's and M8's — and Edit ▸
Find over the page, a go-to prompt and a word count, which came after M11; and a notice banner for an edit the core refuses, which beeps instead.

### What M7 built

Built and type-checked, not yet run. Two hoists came first. `grind_text::format::{here, apply}`
is where a formatting bar reads and writes — the selection's agreed style, or with nothing
selected the style the next character typed takes — which `ui_text_gtk`, `ui_web` and
`ui_win32` each had; hoisting it found that three of them turned a toggle *off* by writing
`normal` or `none` where the GNOME window removed the property, and every shell now removes it.
`grind_sheet::format::Preset` is the number picker's nine rows, out of `ui_win32`.

| File | Half | What it is |
|---|---|---|
| `menu.rs` | portable | Format ▸ Font, Text, Text Color, Background Color, Number and Paragraph, then Clear Formatting — TextEdit's and Pages' shape, the platform's ⌘B/⌘I/⌘U/⌘T/⇧⌘C, and `Command::applies`, which kind of document each verb means something in |
| `sheet/format.rs` `text/format.rs` | portable | What each command writes on either pane and when its item is ticked, over the cores' vocabularies; and the font panel's answer read as the difference between the font it was shown and the one it gave back |
| `tools.rs` | portable | The toolbar as data, with decision 3's test made mechanical: every segment and pop-up row is a menu command with the same title, every colour well has its colour submenu, and nothing on it is a verb |
| `formatting.rs` | Mac | Both panes' `format`, `changeFont:`, `changeColor:` and Show Fonts; `Formats`, the one trait the toolbar speaks to |
| `toolbar.rs` | Mac | The `NSToolbar`: segmented controls, pop-ups and colour wells, every one reading the selection before it shows |
| `app.rs` | Mac | Format commands reach the front document; `validateMenuItem:` greys and ticks |
| `drive.rs` | both | A ⌘-key or a menu step sends its action along AppKit's documented path from the window when the window cannot be key, which on a runner it never is |

Two bugs M7 found in what M3–M5 built, both of the kind only a runner shows: the delegate found
the front document through `currentDocument`, which is nil when no window is main — so Go To,
and every Format command, reached nothing in a drive — and a drive's ⌘-key went to a nil
target that starts at the key window, so Copy, Save and Undo did nothing either. Both are
very likely why `artifacts.yml`'s `macos` job has been red since M3.

What M7 leaves: a cell's font family (the font panel can choose one and a `CellStyle` cannot
hold it, so it is left out rather than approximated); borders; a paragraph kind over several
blocks is one ⌘Z per block; and the toolbar's controls have not been seen, only compiled.

### What M8 built

Built and type-checked, not yet run. Two more hoists came first, each a copy the Mac would
otherwise have made: `grind_sheet::formula::assist` gained the offer machine (`Assist`), the
band's runs, the friendly formula-bar line and the function list's rows, out of `ui_win32`,
which keeps only which keys steer it; and `CellRole::hue` is which colour each role is, the
table `ui_sheet_gtk` and `ui_win32` each carried.

| File | Half | What it is |
|---|---|---|
| `sheet/assist.rs` | portable | Which selectors steer a list of offers — `insertTab:` takes one, the vertical arrows step, Esc closes it for the word, and Return still commits — and what the formula read-out shows at rest and while typing |
| `editor.rs` | Mac | A one-line band under the cell editor with the offers or the signature, the accent where the core says `Strong`; an accepted offer goes in through the field editor, so it is one step of the field's own undo |
| `app.rs` `menu.rs` | both | View ▸ Friendly Formulas (on by default) and Explain Formula, Insert ▸ Function… (all 110 by name and plain-English name), View ▸ Cell Roles, Names and Show Source (⌥⌘U) |
| `sheet/paint.rs` `text/paint.rs` | portable | The role marker in a margin reserved at a cell's leading edge and a muted outline round each defined name; on a page, each bookmark named in the margin beside its line, never over the ink |
| `places.rs` | portable | The sidebar's rows for either document — sheets, names and Problems; outline, bookmarks and Problems — and where each goes, a Problems row to another sheet if that is where the finding is |
| `sidebar.rs` | Mac | One source list over those rows for both panes, so a page has a sidebar too; re-read when the document changes and not when the cursor moves |
| `code.rs` | portable | The source pane's lines in UTF-16 and its colours, from the projection's own token map |
| `source_pane.rs` | Mac | The projection in a read-only `NSTextView` in a trailing inspector item, the selection's line marked and a click on a line going where it projects |
| `drive.rs` | both | `sidebar <text>`, which chooses a row the way a click does |

What M8 leaves: the signature band follows the text, so a caret moved by an arrow without
typing shows the old argument until the next keystroke; point mode (arrows building a reference
into a half-typed formula), which is the GNOME window's alone; the formula read-out is still a
read-out rather than a second editor; every change re-lints the whole document for the Problems
section (a `ponytail:` in `sidebar.rs`); and the source pane is read-only, as every shell's is.

### What M9 built

Built and type-checked, not yet run.

| File | Half | What it is |
|---|---|---|
| `welcome.rs` | portable | Decision 6's window: the three choices, each a menu item's own action; the layout, the hit test and the keys; the system's recent documents under the cards; and a frame as `Op`s, so `--render-to` with nothing named draws it |
| `welcome_window.rs` | Mac | The view that draws that frame in the system's colours and answers the keys by selector; shown at a launch with nothing named, on a Dock click with no window open, and from Window ▸ Welcome to Grind (⇧⌘1) |
| `menu.rs` | portable | The grid's and the page's context menus as tables, every row held by a test to a menu-bar row with the same title, key and action |
| `grid_view.rs` `page_view.rs` | Mac | `menuForEvent:` from those tables, the selection moved to what was clicked first; the page as an accessibility text area, and an announcement on every move on the grid |
| `a11y.rs` | portable | The words the floor says: the page's value, its selected range in UTF-16, the caret's line, and what a move on the grid announces |
| `drive.rs` | both | `a11y`, which asks the key view what VoiceOver would ask, in-process, and prints the answers |

**`tests/appkit.rs` was not built, and the drive is its replacement.** It was planned as a
`harness = false` integration test reading the accessibility attributes back on the main thread,
but `grind-mac` is a binary crate, and an integration test of a binary can only run it, not reach
into its views. A drive already runs the real application on the main thread with a document
open, so the read-back is a drive step and `mac-frames.sh` asserts on the line it prints — the
same assertion, made where the views exist.

What M9 leaves: Open Recent needed nothing — `NSDocumentController` fills it, which M2's
`clearRecentDocuments:` item was for; a line of the page's accessibility value is a block rather
than a laid-out line (a `ponytail:` in `a11y.rs`, `accesskit_macos` the upgrade); and the grid is
announced rather than described cell by cell, which is decision 10's floor and not its ceiling.

### What M10 built

Built, and not yet run: every check is in `.github/scripts/mac-bundle.sh`, which `artifacts.yml`'s
`macos` job runs after the frames — and whether or not they passed, since its checks are about
the binary.

| File | Half | What it is |
|---|---|---|
| `plist.rs` | portable | The Info.plist from one table of nine types — `assoc.rs`'s, offered never taken: every type **Alternate** but `.grind`, **Owner** and exported conforming to `public.plain-text`; ODF forms as Editor, imports as Viewer. Printed by `grind-mac --info-plist`; tests hold each extension to a form this build reads and the floor to the deployment target |
| `document.rs` | Mac | Declared types would have given a Save the package's extension; the document answers it — its own, or the flat form for an untitled one (`doc/flat-first.md`) — and the panel keeps what is typed |
| `data/grind.svg` `data/make-icns.py` `data/Grind.icns` | portable | The suite's mark on the macOS icon grid, rasterised and packed into eleven sizes with no Mac tool, reproducibly, and checked in as `grind.ico` is |
| `main.rs` | both | `--handlers <file>`, what LaunchServices offers for a file and its default, asked of `NSWorkspace` |
| `mac-bundle.sh` `artifacts.yml` | CI | The x86_64 slice, `lipo`, the bundle, the ad-hoc signature, the DMG and its upload, and the checks: `plutil -lint`, `codesign --verify --strict`, `otool -L`, `vtool`'s 15.0, LaunchServices offering without taking, and `spctl` rejecting, as expected |

What M10 leaves: **the tap**. A Homebrew cask points at a published DMG by URL and checksum, and
this project publishes nothing yet — the DMG is a workflow artifact — so the tap, with decision
12's Open Anyway step written into its caveats, waits for the first release. Quick Look's preview
of a `.grind` is claimed by the declaration and not yet seen.

### What M11 built

| File | What it is |
|---|---|
| `scripts/mac-remote.sh` | `renders` fetches the latest `artifacts.yml` run's frames and every annotation its scripts left; `drive <script> [document] [branch]` dispatches `mac-drive.yml`, waits, prints the transcript and fetches the snapshots; `session` asks for `mac-session.yml`. Needs `gh` signed in |
| `.github/workflows/mac-drive.yml` | A script against the newest DMG `artifacts.yml` left on a branch — compiling nothing — with the transcript uploaded and annotated, and the inputs reaching the shell only through its environment |
| `.github/workflows/mac-session.yml` | M12's session: dispatch only, at most an hour, three secrets checked before anything happens |
| `doc/feature-matrix.md` | The Mac column, read out of `ui_mac` through M10, whose ● means *written* until a run says *working* |
| `ui_mac/src/document.rs` `app.rs` | A drive started with `--sheet` or `--text` is handed its new document, so `save`, `click` and `sidebar` act on it |

**The loop's whole design is the lesson of M2–M10**: everything that has to be read back from a
runner is an *annotation*, because the public API serves annotations to anyone while a job's log
and its artifacts need credentials. `mac-frames.sh`, `mac-bundle.sh` and `mac-drive.yml` all
report that way, and the probe's answers reached *Evidence* that way.

### After M11: the named gaps, closed one at a time

With M12 waiting on a person, the work turned to what each milestone named as left. Closed so far:
**Find and Replace on the page** (one find bar over a `Findable` trait both panes answer, with a
status line of its own, since a page window has no banner); **Go To on a page** (⌘L asks for
`p12`, `#intro` or `§2.1` in an alert); **Edit ▸ Recalculate** on ⌘=, Excel for Mac's key; a
page's **word count** in its window's subtitle; **pictures drawn on the page** — `NSImage`
decodes them, `text/picture.rs` fits each to the column at no more than its own size with its
caption under it (`ui_text_gtk`'s rule, so a figure takes the same room in both windows), and
bytes nothing reads stay outlined — and **Insert ▸ Picture…**, an open panel whose file is
embedded in a paragraph of its own, refused when its signature is no picture's (`picture::mime`); **charts drawn on the grid** — `sheet/chart.rs` turns each into `Op`s (bars, a line as one
`Op::Path`, a pie as one path per slice from `grind_sheet::pie_slices`), scaled by `axis_ticks`
and coloured by `effective_color` as the GNOME window and the browser are, with its title and
legend placed on *measured* text rather than the browser's estimate; read-only, and the value
axis' title set level above the axis rather than turned along it; **cell borders**, drawn per edge centred on the grid line they replace, and Format ▸ Borders
writing the terminal's and the browser's hairline; **tracks resized from the header bands** — the
resize cursors over every edge, a drag drawn live from `Sizes::with` and written once when the
button comes up (every selected whole track with it, one undo step), and a double-click fitting a
column to its widest text or giving a row back to its content (`sheet/resize.rs`); **rows grown to their content** — `Grid::measured`, a row with no
height of its own as tall as its tallest wrapping or larger-faced cell through
`grind_core::layout::wrap`, the GNOME window's L3 rule and bound; **a caret that blinks**, lit for a whole period after every key and
click, its rectangle alone redrawn on each tick; **point mode** — typing `=SUM(` and pressing an arrow points at a
cell instead of committing, Shift grows the range, the pointed cells are outlined in the system's
orange, and typing anything ends it; the predicate is `formula::assist::ref_eligible`, hoisted
out of the GNOME window for it, and a drive in `mac-frames.sh` checks the formula it writes.
Pointing with the mouse and F4's `$` cycle are not built: ⌘T, Excel for Mac's key for the cycle,
is the font panel's here; **Insert ▸ Chart**, the selection's table charted beside it by
`App::suggest_chart` in one step, with no dialog, and a chart's own context menu — its title,
bar, line or pie, where its legend goes, and Delete Chart (`App::edit_chart`, `App::remove_chart`)
— and dragged to move it, drawn where the pointer has it and written once as the button comes up
(`App::reshape_chart`); every Format ▸ Number item carrying **a live sample** of the active
cell in its format as the item's subtitle (`App::shown_as`, macOS 15's `NSMenuItem.subtitle`); File ▸ **Import CSV…** into the open sheet at the active cell, and
Export as CSV… writing TSV when the name says `.tsv`; **the autofilter** — Edit ▸ Filter (⇧⌘F, Excel for Mac's key)
over the selection or from one cell to the end of the used part, a button drawn in each heading
and filled in the accent while its field has a condition, and a click on one listing the field's
values as a menu (`filter::offered`'s order), ticked while shown, plus Show All; **every paragraph kind a page draws** — Title, Subtitle and
Heading 4–6 beside Body, Heading 1–3 and List Item, the two names taken off again only when this
page put them on, as the GNOME window does; Edit ▸ **Copy Value** (`App::value_text` through `clip::rect_text`)
and **Formula to Value** (`App::clear_formula`, LibreOffice's name, the first window with it); Edit ▸ **Evaluate…** (`App::preview` at the active cell, display syntax in, an alert out)
and Format ▸ Number ▸ **Document Locale…** (`App::set_locale`, a tag or nothing); **a defined name's context menu** in the sidebar — Rename… (every
use following, `App::rename_name`), Redefine…, Inline Everywhere (`App::inline_name`) and Delete,
the first window with the first and third; **View ▸ Formulas** (⌃⌘\`), every formula shown in its cell in
display syntax rather than its value, drawn and never written — the first window with it; and with View ▸ Names on, the formula read-out **reads a formula
through its names** (`App::named_formula`); Edit ▸ Fill ▸ **Across Selection**, the active cell into the whole
selection in one `App::fill`; **View ▸ Calculations…**, every formula in the document listed in
the sidebar as a section of its own — address, formula and value, each a jump — narrowed by what
it asked for (`App::calculations`, the GNOME window's dialog as a list of places); Format ▸ Paragraph ▸ **Move Up, Move Down and Delete Paragraph**
over the paragraphs the selection touches, one undo step each (`App::move_blocks`, `App::delete`); and Format ▸ Paragraph ▸ **Style…**, a named paragraph style on them
(`App::set_style`), kept and not interpreted unless it is Title or Subtitle; and `canAsynchronouslyWriteToURL:` answering NO
where *Risks* says it does, rather than by `NSDocument`'s default. `doc/feature-matrix.md`'s Mac
column carries each.

## Conventions made mechanical

"Follow the platform's conventions" is a promise that rots in a shell nobody here can run. So
as much of it as possible is a test, and most of those tests are portable:

- Every `Command` is in exactly one menu, and every toolbar item has a menu twin.
- Standard items use **the standard selectors** (`copy:`, `undo:`, `performClose:`,
  `saveDocument:`, `performFindPanelAction:`, `orderFrontFontPanel:`, `toggleSidebar:` …). That
  is what makes Services, Help ▸ Search, the Edit menu's system additions and VoiceOver's
  command names work. A misspelt selector is a grey menu item and nothing else, which is why
  this is a test and not a review note.
- No two items share a key equivalent, and **none takes a shortcut the system reserves**: ⌘Space,
  ⌘Tab, ⌘\`, ⌘H, ⌥⌘H, ⌘M, ⌘Q, ⌃⌘Q, ⌥⌘Esc, ⇧⌘3/4/5, ⌃⌘Space and ⌃⌘F.
- The shortcuts that are the platform's are the platform's: ⌘N, ⌘O, ⌘S, ⇧⌘S, ⌘W, ⌘Z/⇧⌘Z,
  ⌘F/⌘G/⇧⌘G/⌘E, ⌘B/⌘I/⌘U, ⌘T for the font panel, ⇧⌘C for colours, ⌃⌘S for the sidebar. Grind's
  own follow precedent where there is one: Show Source ⌥⌘U, Go To ⌘L.
- A title ends in "…" if and only if its command asks for more before acting. Titles are in
  title case.
- The application, Window and Help menus exist and are the ones AppKit is told about
  (`windowsMenu`, `helpMenu`), which is what gives the window list and the Help search.
- Every notice spells a key as ⌘, ⌥, ⇧ or ⌃ — never "Ctrl".
- Every extension `plist.rs` declares is one `Form` and `kind` accept, and every one has a rank.
- **Pinned to the platform, on the runner only**: every selector the system's
  `StandardKeyBinding.dict` binds is answered or named in `keys.rs`, and every menu item's action
  resolves (`NSApp.targetForAction:`) and validates with a document open.

## CI

**One build per platform** still holds (`CLAUDE.md`, "CI, and the one rule that shapes it"):

- **`macos.yml` → `check-from-linux`** (ubuntu): `check`, `clippy -D warnings` and
  `doc -D warnings` for `aarch64-apple-darwin` and `x86_64-apple-darwin`, then the portable
  tests. The fast signal, with its own cache key. *Built in M0.*
- **`macos.yml` → `probe`** (`macos-15`, `macos-26`): M0's measurements, uploaded. *Built in
  M0, retired in M2.*
- **`artifacts.yml` → `macos`** (`macos-26`): one release build of both slices, then everything
  that has something to say about the binary, as later steps. That is `cargo test -p grind-mac`
  (the `[M]` unit tests), the bundle, M10's checks, the renders twice each, the
  drive scripts in `ui_mac/tests/drives/`, the DMG and its upload, the size record, and
  `cargo bloat` **last**, for the reason `CLAUDE.md` gives. *From M2.*
- **`artifacts.yml` → `macos-floor`** (`macos-15`, `needs: macos`): the same `.app`, downloaded
  and never rebuilt, rendered and driven on the floor. *From M2.*
- **`screenshots.yml`** gains a Mac job reading that `.app`; it compiles nothing. *From M2.*
- **`mac-drive.yml`** (`workflow_dispatch`, a drive script as its input) runs the latest `.app`
  against the script and uploads the transcript and the PNGs — and emits the transcript as an
  annotation, which a session with no credentials can read. It is the Linux side's "see it
  working". *Built in M11.*
- **`mac-session.yml`** (`workflow_dispatch` only; never on a push or a pull request; at most an
  hour) is M12's screen-sharing session. The runner enables Screen Sharing with a password taken
  from a secret and joins a private network as an ephemeral node, using an auth key that is also
  a secret, so the session is reachable from the user's own machines and nowhere else. **The
  password is never printed**: a public repository's logs are public. The private network is
  Tailscale's, through an OAuth client rather than a reusable key. *Built in M11, not yet run;
  used in M12.*

The cost: macOS minutes are free on public repositories and billed at ten times the Linux rate
on private ones. `check-from-linux` carries the everyday signal on the cheapest runner there is.

## What it will not do

Named up front, as R10 requires. Each is revisited at the milestone that could change it.

- **No Mac App Store** (`doc/plan.md`), and **no notarization yet** (decision 12).
- **No command palette**: the Help menu's search is the platform's (decision 3).
- **No chart drawing**. Charts are read, kept and written back, as in `ui_win32` and `grind-tui`.
- **Pagination, printing and RTL**: gated in `doc/not-doing.md` and `doc/text-layout.md`, and not
  this shell's to open. File ▸ Print is therefore absent rather than a stub, and Page Setup with
  it.
- **No Settings window.** There is nothing to set that the document or the system does not
  already own, and a Mac application with an empty Settings window is worse than one without.
  ⌘, stays unbound until there is something to put behind it.
- **No iCloud Drive integration beyond what `NSDocument` gives any document.**

## What the runner cannot speak for

The list M12 exists to shorten:

- Trackpad scrolling momentum and rubber-banding **feel**. The runner can show that the scroll
  view is an `NSScrollView`, not how it feels.
- VoiceOver's actual speech. The runner can read back the attributes (decision 10), not whether
  what they make VoiceOver say is any good.
- A real CJK input method. M6 attempts one on the runner through `TISSelectInputSource`, falling
  back to driving `NSTextInputClient` directly, which proves the protocol rather than the
  experience.
- Liquid Glass materials in a snapshot. A cached frame view may not composite a blur the way the
  window server does, so the toolbar in a drive's PNG is only indicative.
- Two displays of different scale, and a window dragged between them.
- Gatekeeper on a genuinely clean machine. `spctl` on the runner gives the verdict. A clean
  machine and a real download give the experience.

## Risks

- **The runner has no GUI session.** Then decision 9's `--drive` half is gone, M2 onward can
  only render, and M12 carries more. The probe measures this before anything depends on it.
- **objc2's API moves.** It is 0.x, and its framework crates are generated from each Xcode's SDK.
  The mitigation is `ui_win32`'s: the whole AppKit half is type-checked from Linux in about a
  minute on every change, so a breaking bump shows up as a failed check, not as a surprise on the
  runner.
- **`unsafe`.** objc2 makes most calls safe, and marks unsafe exactly the ones the Objective-C
  contract cannot check (raw pointers, generic dictionaries, ownership on close). The rule is
  `ui_win32`'s: every `unsafe` block carries a `SAFETY:` line saying which contract it upholds,
  and they live only in `[M]` files.
- **`NSDocument` autosave and the core's observer.** An autosave is a `dataOfType:` call on a
  background queue unless it is told otherwise. This shell answers
  `canAsynchronouslyWriteToURL:` with NO, so every write happens on the main thread, where
  `App` is used. This is simpler than a snapshot, and fast enough because R6's write is a
  splice.
- **Nobody ever runs it on their own Mac.** Then every claim rests on the runner, which is a
  better position than `ui_win32`'s rested on Wine, and still not a person. M12 is the answer,
  and the list above is its size.

## Verification

On this Linux machine, for every change to this crate:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo check  -p grind-mac --all-targets --target aarch64-apple-darwin
cargo clippy -p grind-mac --all-targets --target aarch64-apple-darwin -- -D warnings
cargo clippy -p grind-mac --all-targets --target x86_64-apple-darwin  -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps -p grind-mac --target aarch64-apple-darwin
cargo test -p grind-mac
```

On a Mac, if one is to hand, there is nothing to arrange:

```sh
cargo run -p grind-mac -- book.fods
cargo run -p grind-mac --example probe -- /tmp/probe    # M0's measurements, by hand
```

## Evidence

Measured on 2026-09-29, on the Linux development machine, before any AppKit code existed:

| Claim | How it was checked |
|---|---|
| `grind-core`, `grind-sheet`, `grind-text` and `grind-xlsx` type-check for macOS on Linux, with no SDK | `cargo check --target aarch64-apple-darwin`: clean, 17.7 s cold; `--target x86_64-apple-darwin`: clean, 11.6 s |
| The whole CLI type-checks for macOS | `cargo check -p grind-cli --target aarch64-apple-darwin`: clean, 29.5 s |
| objc2's framework crates are all-features by default, and that is expensive | With defaults, 16 crates join `Cargo.lock` (313 → 330, `grind-mac` included), CloudKit, CoreData, Metal and CoreImage bindings among them, and a cold check of the stack takes **53.8 s** |
| Gated to the classes used, it is cheap | `default-features = false` with the probe's features: **9** crates (313 → 323), and **9.2 s** cold — about what the `windows` crate cost `ui_win32` (10 crates, 5.0 s) |
| AppKit, CoreGraphics and CoreText code type-checks **and lints** from Linux | The probe (`examples/probe.rs`): `cargo clippy --all-targets -D warnings` clean for both Apple targets and the host. It was written against the crates' generated sources, and never run here |
| The portable half tests on Linux | `cargo test -p grind-mac`: 22 passed — argument handling including AppKit's own launch arguments, the type flags, the bytes deciding over the extension, and the workbook and CSV sniffs |
| AppKit's launch arguments do not become files | `grind-mac -AppleLanguages '(de)' examples/quote.grind` resolves to the projection, a spreadsheet, and to nothing else |

**The first run of `macos.yml`'s `probe` job** was run 36620879811, on the M0 commit, and it
was green on both `macos-15` and `macos-26`. Green settles only what the job *asserts*: the core
question below (the step ends in `test … = 9`), and that CoreText draws into a bitmap with no
application object and a window, a content view and a key event can all be built — the probe
exits non-zero otherwise. Everything else it *prints*, and from run 36679496310 (on `d85d6a0`,
2026-09-30) the Summary step emits each answer file as a workflow annotation, which the public
API serves to anyone (`/repos/fwilhe2/grind/check-runs/<job>/annotations`). That run answered
every row, identically on macOS 15.7.9 and 26.6.2 (arm64) unless a row says otherwise:

| Question | Answer | What depends on it |
|---|---|---|
| Does the suite's core run on macOS — `grind sheet set`, `recalc`, `view`, `lint` — and does `=SUM` give 9? | **Yes**, on both, asserted by the step | Everything; `ui_win32`'s W0 found a stack overflow this way |
| Does CoreText draw into a `CGBitmapContext` with no application object, and are two renders the same bytes? | **Yes**, and yes (`headless-identical: true`; 8,983 bytes on 15, 9,023 on 26 — the same drawing, not the same pixels, across versions) | Decision 9's `--render-to` |
| What are the caret offsets of `é`, `e◌́`, a ZWJ family and `क्षि`? Are they one stop each? | **One stop each.** `é` is `[0, 12.62]` and `e◌́` `[0, 0, 12.62]` — the same width; the family (5 characters, 8 UTF-16 units) is `0` at every unit and `27.00` after the last; the conjunct (4 characters) `0` inside and `23.40` after it (`23.30` on 26). `fold` makes each one advance, on its last character. `metrics.rs`'s Mac-only tests now hold the page's own `Metrics` to this on every run | Decision 4 |
| Is there a screen, does a window become visible and key, and does the application become active? | **One screen, at a backing scale of 1** — not Retina. The window **becomes visible but never key**, and the application **never active**: a runner's session has no user to hand focus to | Decision 9's `--drive`, and M2 onward. A drive must not assume a key window: `drive.rs` falls back to the first window, sends a ⌘-key's action along AppKit's own path from it (M7), and the delegate finds the front document without a main window. Anything drawn only while the window is key — the page's caret — is absent from a drive's snapshots, which is the platform being honest rather than a bug. **Since amended**: on run 36816633386 (2026-10-01) both runners report the application active and the window key at a drive's start — and a drive's arrow keys still moved nothing, which is why its first line now names the first responder as well |
| Does caching the frame view give a picture of the window? | **Yes** (7,119 and 7,320 bytes) | `--drive`'s `snap` |
| Do synthesized key events reach a focused `NSTextField`? | **Yes** — `"hi"` typed, in a window that was not key | `--drive` itself |
| Does `screencapture` work from a job step? | **Yes** (about 79 KB) | Nothing depends on it; it is a second opinion |
