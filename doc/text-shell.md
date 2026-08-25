<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The word processor's windows — S9 and S10

What the GTK and browser shells for `grind text` do, what they deliberately do not, and what
building them proved about the layout decision. Normative for those two milestones the way
`doc/sheet-shell.md` is for phase 9; `doc/suite.md` is still normative for the phase around
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
| Selection | Shift+arrow, Shift+click, dragging the mouse; typing or Enter over one replaces it | — (named gap) |
| Formatting | a toolbar (Bold/Italic/Underline/Strikethrough) over the selection, `App::char_style`/`set_char_style`; `Title`/`Subtitle` paragraphs and every run's own formatting are drawn, not only measured | — (named gap) |
| Images | `grind text image` inserts one (`App::insert_image`); a block that is a picture — with or without a caption read alongside it — is decoded and drawn fit-to-column, the caption wrapped underneath, both sized into the flow from the picture and the caption rather than a line of text; reads either the schema's `office:binary-data` or a package's own `xlink:href` part | — (named gap) |
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
measured every run with the *default* character style when these shells were written, because a
run carried a style *name* and nothing else (`doc/text-core.md`). A shell that drew a heading
larger than it measured it would put every caret in that heading in the wrong place. Neither
shell does: it knows the block's kind, so it hands the core a provider already set to *that
block's* font, and the core does arithmetic in whatever unit it is answered in. The heading
scale is the same six numbers in both (`HEADING_SCALE`), so a document has one shape in both
windows — and there is a test in `ui_web` that says so.

> Half of that has since been fixed in the core rather than in a shell: a run now carries direct
> formatting, so `lay_out` projects each run's `CharStyle` into the four metric properties and
> hands *those* to the provider, per fragment. A bold word is measured bold without a shell
> knowing. The block-level half is unchanged and still the shell's — a heading is a *paragraph*
> style, and the paragraph family is still gated.
>
> **`grind-text-gtk` now draws what the core measures, the other shell does not yet.** A line's
> Pango attribute list is built from the block's own `RunView`s (`ui_text_gtk/src/metrics.rs`'s
> `run_attributes`, one `pango::AttrInt`/`AttrColor` per bold/italic/underline/strikethrough run,
> clipped to the line and converted from the model's character offsets to the byte offsets Pango
> attributes are measured against) rather than one plain string per line — so a bold word in a
> file that already laid out bold now paints bold, with no change to the arithmetic that placed
> it there. `Title` and `Subtitle` get the same treatment one level up: `Faces::of` checks the
> block's named style before its kind, because both are `BlockKind::Paragraph` with nothing else
> to key a face off, and are given their own (larger, and italic for `Subtitle`) faces alongside
> the six heading ones. `ui_web`'s pane has neither yet — a bold run there still paints plain,
> and a `Title` paragraph draws as body text — which is the browser shell's own instance of this
> same gap, unclosed.

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

**Both shells.** No copy, cut or paste — `App::erase` takes two carets, and a selection can now
name them in `grind-text-gtk`, but nothing yet puts either end on a clipboard. No find/replace UI
(`grind text find`/`replace` exist). No lists UI: a list item read from a file draws with its
bullet and its indent, and nothing creates or renests one. No tables or footnotes, because the
core has none — `text:page-number` and the other named fields (`text:date`, `text:title`,
`text:file-name`) are also out, both for the same reason `doc/text-core.md` gives. No pages, no
print, no zoom. No RTL — excluded by decision in `doc/text-layout.md`. Tab stops are measured per
run rather than per line, so a line with several tabs drifts from where a word processor would
put them. A named-style picker stays out: a run's *named* style (as opposed to its direct
formatting) is a name this build keeps and does not interpret, and turning `Emphasis` into a set
of checkboxes would throw that structure away to draw it (`doc/text-core.md`).

**`grind-web`'s document pane only, for images.** `grind-text-gtk` decodes and draws one — the
CLI's own `--render-to` proof is `Earthrise.fodt` and `Earthrise.odt`, a real photograph and
caption LibreOffice wrote in both physical forms, opening with the picture and its caption both
visible rather than either dropped. The browser pane still has neither the model call nor the
drawing; the second is now most of the work, since `App::insert_image` and the reader/writer
support underneath it are shared by every shell already. Two things are true in `grind-text-gtk`
alone even once the browser catches up: an image sitting *mid-sentence* — the
`text:anchor-type="char"` case, as opposed to a paragraph that is only a picture (optionally
followed by its caption's text, which reads the same way) — still draws as the placeholder
character (`\u{fffc}`) rather than the picture, because nothing here lays inline content out
around one yet; and an image is fit to the column width on its own terms, ignoring the
document's own `svg:width`/`svg:height` (`RunView::image`'s `ImageView::width`/`height`), because
turning an ODF length into device pixels needs a resolution this shell does not otherwise track.
A package's picture referenced by `xlink:href` into `Pictures/` (as opposed to embedded
`office:binary-data`) now resolves at read time in every shell (`doc/odt-format.md`'s "The image
itself may be a reference rather than bytes"), so this is no longer a gap.

**`grind-web`'s document pane only, now.** No selection — no shift-click, no shift-arrow, no
dragging — and no styling UI, which used to be a *both-shells* gap blocked on the model carrying
only a run's style name. That blocker is gone (`App::set_char_style`/`char_style`,
`doc/text-core.md`) and `grind-text-gtk` closed both halves of it: an anchor alongside the caret
(`Doc::selection`, Shift+arrow and Shift+click extend it, dragging with the button down grows it
continuously through `GestureDrag`'s own `drag-begin`/`drag-update`, typing or Enter over one
erases it first the way every other editor's Shift key does), a toolbar of four toggles that
read the selection's common formatting and write through `set_char_style`, and the drawing fix
in the section above so a toggle's effect is visible rather than only present in the file. The
browser pane has none of it yet — the first candidate when it grows, the same way the outline
dialog and go-to-address field are (below).

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
spreadsheet's own gap list in `doc/sheet-shell.md` still applies to the other one.

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
