<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# The macOS shell

The plan for `ui_mac/`: crate `grind-mac`, binary `grind-mac`, shipped as **`Grind.app`**.
It also records the decisions behind the plan. It is normative for that directory the way
`doc/windows-shell.md` is for `ui_win32/` and `doc/tui-shell.md` is for `ui_tui/`.
`doc/plan.md` reserved the directory name long before anything was in it, and ruled out the Mac
App Store (GPLv3 §6).

**Status: built**, and run on a Mac now and then; §"What is not built" is what is left.

## In one line

**An AppKit application over `grind-core`, holding both document types. It is written in Rust,
developed on a machine that is not a Mac, and built, run and looked at on GitHub's macOS
runners.**

## The problem this plan is shaped by

This shell is developed on a machine that is not a Mac. So the question this document answers
first is not "how should a Mac shell look" but **how much of a Mac shell can be built and tested
without one**. The Windows shell asked the same question and answered it with a portable
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
| **The sidebar** (`NSSplitViewController`, a source list) | *where you can go*: sheets and defined names, or headings and bookmarks, plus a **Problems** section whose every row is a jump | Win32's go-to, outline and lint dialogs |
| **The titlebar accessory** (`NSTitlebarAccessoryViewController`, bottom) | the name box and the formula field, which are the two read-outs of *where* and *what* | Win32's strip |
| **Context menus** | built from the menu table, so a row and its menu-bar twin cannot disagree | — |
| **The source pane** | The projection (`doc/dsl.md` §6), as a trailing split item with a read-only `NSTextView`, toggled by View ▸ Show Source (⌥⌘U, Safari's key for the same idea) | Win32's modal listbox |

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
autosave; `doc/feature-matrix.md` §9 and `doc/not-doing.md` §3 say "the Mac only, by platform
convention". What makes it safe here rather than reckless:

- **An untouched document is never written.** `NSDocument` autosaves only a document with
  changes, and the observer is registered after the load, so opening a file is not an edit.
  M4's exit criterion asserts the bytes *and* the modification time.
- **An imported workbook, Word document or CSV is untitled.** It opens as a new document under
  `grind_xlsx::suggested_name` (or `grind_docx::suggested_name`) with no file URL, so there is nothing to autosave *into*, and an
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

## The page model

The page view shows the document's **printed pages** (`doc/page-model.md`): `grind_print`
sets them, each is drawn from its raster as an `Op::Pixels` (`render.rs` makes a `CGImage` over
the pixels), and the selection, the dots, bookmark names, marked text and the caret go over them
from `grind_print::view`. A view point is a page's point; the scroll view's magnification is the
zoom, and a page is rasterised at the screen's pixels times it, so it stays sharp. Motions by lines
are the pages' (`grind_text::caret::Lines`, over `grind_print::place`), a click is `view::Shown::hit`, and
`firstRectForCharacterRange:` is the caret's stroke on its page. CoreText `Metrics`, the flow and
the inline composition are gone; marked text is drawn over the page at the caret until committed
(a `ponytail:`). `--render-to` draws the same frame. Type-checked and linted here; the runner's
frames and drives are what show it.

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

## What is not built

The Mac column of `doc/feature-matrix.md` is what the shell does. Beyond its gaps:

* **The Homebrew tap.** A cask points at a published DMG, and nothing is published yet: the
  DMG is a workflow artifact.
* **F4's `$` cycle** while a formula is edited.
* **A cell's font family** from the font panel: a `CellStyle` carries none.
* **The UX pass over a screen-sharing session** (`mac-session.yml`), which needs its secrets set.

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

**One build per platform** still holds (`CLAUDE.md`, "CI"):

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
  Tailscale's, through an OAuth client rather than a reusable key.

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
