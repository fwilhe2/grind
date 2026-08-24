<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The word processor's windows — S9 and S10

What the GTK and browser shells for `grind text` do, what they deliberately do not, and what
building them proved about the layout decision. Normative for those two milestones the way
`doc/gtk-shell.md` is for phase 9; `doc/suite.md` is still normative for the phase around
them.

Both shells are **minimal on purpose**. `doc/suite.md` sizes S9 as "large" and describes a
milestone-by-milestone shell mirroring the spreadsheet's nine. This is not that: it is the
floor R10 asks for — *a document type that only has a window is a product decision nobody
made*, and so is one that has no window. What each shell does not do is listed below rather
than discovered, and every line of it is a thing the CLI can already do.

## What is built

| | `grind-text-gtk` (S9) | `grind-web`'s document pane (S10) |
|---|---|---|
| Open, save | `.odt` and `.fodt`, file dialog and recent files | bytes in through the File API, out as a download — no path anywhere (rule 5) |
| Draw | custom widget, `snapshot()`, Pango | one `<div>` per **laid-out line**, no `contenteditable` |
| Caret motion | arrows, Home/End, Page, Ctrl+Home/End — all answered by the core | the same set |
| Editing | type, Enter, Backspace, Delete | the same four |
| Undo/redo | `App::undo`, in the core | the same, on the shared toolbar |
| Structure | outline dialog, go-to-address popover, heading level 0–3 on Ctrl+0…3 | — (named gap) |
| Cross-app | a `.ods` opens a banner: *"This is a spreadsheet"* + **Open in Sheet** | one bundle, so the other pane simply opens |
| Assertable output | `--render-to <png>`, one frame then exit | `ui_web/smoke.js`, jsdom, no browser |

Both are renderers that own nothing: every paint reads `App::get_viewport` and
`App::layout_block` and throws the result away. Neither has a text buffer, and neither has a
history — `GtkTextView`/`contenteditable` were declined for exactly that reason, in the
module docs of `ui_text_gtk/src/view.rs` and `ui_web/src/text/mod.rs`.

## What building them proved, and what it cost

**Path C works, and the terminal was not a special case.** `doc/text-layout.md` chose to put
line breaking in `grind-core` behind a `Metrics` trait so that three shells could not
disagree about where Down-arrow goes. There are now three implementations of that trait —
character cells (`unicode-width`), Pango, and the browser's canvas — and none of them needed
a change to the engine. The GTK shell's is about forty lines, the browser's about thirty.

**A shell may choose the face, and that is what makes headings work.** `grind_text::lay_out`
measures every run with the *default* character style, because a run's style is a name and
style definitions are not read yet (`doc/text-core.md`). A shell that drew a heading larger
than it measured it would put every caret in that heading in the wrong place. Neither shell
does: it knows the block's kind, so it hands the core a provider already set to *that block's*
font, and the core does arithmetic in whatever unit it is answered in. The heading scale is
the same six numbers in both (`HEADING_SCALE`), so a document has one shape in both windows —
and there is a test in `ui_web` that says so.

**An empty paragraph was one unit tall.** `layout::wrap` takes a line's height from the
fragments it was given, and a block with no runs has none, so the height fell back to `1.0` —
a correct line in a terminal and a one-pixel gap on a screen. Found by the GTK shell, fixed in
`grind_text::lay_out` (it now hands the provider one empty fragment), and pinned by
`an_empty_block_is_still_one_line_of_the_metrics_own_height` in `text/tests/app.rs`. This is
the second shell paying for itself.

**`App::caret_line` takes one width and one provider for a motion that may cross into a block
set in a different face.** So Down-arrow out of a heading measures the paragraph below it with
the heading's metrics: invisible mid-line, wrong by a few characters at the ends, and wrong in
the same way in both shells because both are asking the same question. The fix is a *core*
change — a provider looked up per block rather than passed once — and it is written here
rather than worked around in a shell, because working around it would mean a shell doing its
own line arithmetic, which is the thing Path C exists to prevent.

## The gaps, written down

Deferred by decision, not omission. Nothing here is reachable in one shell and missing from
the other unless it says so, and everything here is reachable from the CLI (R9).

**Both shells.** No selection — no shift-click, no shift-arrow, no copy, cut or paste;
`App::erase` takes two carets and nothing yet produces the second. No find/replace UI
(`grind text find`/`replace` exist). No styling UI: no bold, no italic, no named-style picker
— the model carries a run's style *name* and this build reads no style definitions, so a UI
would be offering to write something LibreOffice will not read back (`doc/text-core.md`). No
lists UI: a list item read from a file draws with its bullet and its indent, and nothing
creates or renests one. No tables, footnotes, fields or images, because the core has none. No
pages, no print, no zoom. No RTL — excluded by decision in `doc/text-layout.md`. Tab stops are
measured per run rather than per line, so a line with several tabs drifts from where a word
processor would put them.

**`grind-text-gtk` only.** No `grind-ui` crate: `doc/suite.md` says to extract the shared GTK
plumbing "on evidence, at S9, when the second shell shows the seam", and one *minimal* shell
is not that evidence — this one copied the observer bridge, the `--render-to` harness and the
window-close latch, which is three data points and the right time to look again is when either
shell grows. No `.desktop` file, AppStream metainfo or icon, and no `[package.metadata.deb]`
block: packaging is S11, which does all five packages at once. No shortcuts window. No a11y
beyond the floor (`Accessible::announce` on every caret move, as M9 requires). The document
is re-laid-out in full whenever it or the width changes, so a very long document costs a pass
per resize (`ponytail` in `view.rs`).

**`grind-web` only.** The whole document is in the DOM — no windowing, because a document has
as many blocks as somebody typed and a scroll-position-to-block map only pays for itself on
documents nobody has written yet. Character advances are measured once each and cached, so
kerning between two characters is lost (`ponytail` in `text/mod.rs`); the caret is an element
*in* the line, so the browser still places it against its own kerning, and the cost is at most
a pixel or two in where a line breaks. No input method: a character arrives as a
`KeyboardEvent.key`, so dead keys compose and CJK candidate windows do not. No outline dialog
and no go-to-address field — the two things the GTK shell has that this one does not, and the
first candidates when it grows. Both panes exist in one page and one is `hidden`; the
spreadsheet's own gap list in `doc/gtk-shell.md` still applies to the other one.

## How to see them

```sh
scripts/run.sh text-gtk                     # the sample text document, in a window
cargo run -p grind-text-gtk -- report.fodt
cargo run -p grind-text-gtk -- report.fodt --render-to /tmp/page.png   # one frame, then exit
cargo test -p grind-text-gtk                # geom, keys, and — where there is a display — the widget

ui_web/build.sh release && ui_web/smoke.sh release   # the browser boundary, in jsdom
scripts/run.sh web /tmp/grind-demo/sample.fodt       # served next to the page, opened with ?doc=
```

`cargo test -p grind-text-gtk` runs two kinds of test. `geom.rs` and `keymap.rs` are pure
arithmetic and pure key mapping and always run — that is where the stacking, the scrolling and
every binding are checked. The widget tests in `view.rs` need a display and **skip with a
notice** where there is none, which is where CI runs: `.github/workflows/gtk.yml` installs the
GTK development packages and no compositor, on purpose.
