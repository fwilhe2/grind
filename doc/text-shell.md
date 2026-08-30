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
| Markdown as you type | `**bold**`, `*italic*`, `__underline__`, `~~struck~~`, `` `code` ``, `# `, ``` — `App::type_markdown` | the same, same call. **The two backtick notations are reported not working in any shell** — `TODO:` at the top of `text/src/markdown.rs` |
| Undo/redo | `App::undo`, in the core | the same, on the shared toolbar |
| Structure | outline dialog, go-to-address popover, heading level 0–3 on Ctrl+0…3 | the same three, all inside the Ctrl+K palette (`doc/web-shell.md`) |
| Selection | Shift+arrow, Shift+click, dragging the mouse; typing or Enter over one replaces it | the same |
| Formatting | a toolbar (Bold/Italic/Underline/Strikethrough) over the selection, `App::char_style`/`set_char_style`; `Title`/`Subtitle` paragraphs and every run's own formatting are drawn, not only measured | the same, plus colour and highlight |
| Images | `grind text image` inserts one (`App::insert_image`); a block that is a picture — with or without a caption read alongside it — is decoded and drawn fit-to-column, the caption wrapped underneath, both sized into the flow from the picture and the caption rather than a line of text; reads either the schema's `office:binary-data` or a package's own `xlink:href` part | drawn, as a `data:` URL |
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

**The code view is read-only** (`doc/dsl.md` §6, D9). Ctrl+Shift+U puts the document's projection
on the other page of a `gtk::Stack`; moving the cursor in it puts the caret in the block that line
projects. A stack rather than a paned split, for `doc/web-shell.md`'s reason. `code.rs` is a copy
of `ui_sheet_gtk/src/code.rs` with a different address vocabulary — there is no crate a widget
both GTK shells could use, since `grind-core` may not hold GTK types and neither application crate
may depend on the other (R8); the `ponytail` in both files says what the upgrade is and what
triggers it. The widget half is tested once, in `view.rs`'s `the_widget` harness.

**No lint pane** (`doc/dsl.md` §4.3, D6). `grind lint` checks a document against the rules the
core now holds — a stale cached value, a dead bookmark link, a construct the projection cannot
spell — and every finding carries an address this shell can already go to. What is missing is a
list and a way to click a row; nothing about the rules is a shell's.

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

**Bookmarks are visible now** (`doc/view-modes.md` §3.6, V7). A bookmark is the named-range
analogue and it contributes no characters, which made it the one part of a text document a reader
could not see at all — a zero-width `Run` with nothing on screen to say it was there.
`BlockView::marks` carries them, and every shell draws them: **Show Bookmarks** (Ctrl+Shift+N) in
`grind-text-gtk`, `:names` in `grind-tui`, Ctrl+K → *Show where bookmarks are* in the browser, and
`grind text view --names` down a pipe. Nothing is written by any of them, which is the whole
claim; `--render-to --overlay names` draws a frame with the mode on. This window is the only one
that can say *exactly* where an anchor is — it already has `x_at` for the caret, so the mark gets
a tick at its own offset and its name at the end of the line it falls on. The terminal and the
browser put the name at the end of the line and nothing at the offset, which their own gap lists
say. **Not** a gap either shell closes: nothing here *creates* a bookmark — `grind text name` does
— and there is no list of them to jump between, since the go-to field already takes `#intro`.

**Images.** Both shells decode and draw one now — the CLI's own `--render-to` proof is
`Earthrise.fodt` and `Earthrise.odt`, a real photograph and caption LibreOffice wrote in both
physical forms, opening with the picture and its caption both visible rather than either
dropped; the browser pane draws the same bytes as a `data:` URL. Two things are true in both:
an image sitting *mid-sentence* — the
`text:anchor-type="char"` case, as opposed to a paragraph that is only a picture (optionally
followed by its caption's text, which reads the same way) — still draws as the placeholder
character (`\u{fffc}`) rather than the picture, because nothing here lays inline content out
around one yet; and an image is fit to the column width on its own terms, ignoring the
document's own `svg:width`/`svg:height` (`RunView::image`'s `ImageView::width`/`height`), because
turning an ODF length into device pixels needs a resolution this shell does not otherwise track.
A package's picture referenced by `xlink:href` into `Pictures/` (as opposed to embedded
`office:binary-data`) now resolves at read time in every shell (`doc/odt-format.md`'s "The image
itself may be a reference rather than bytes"), so this is no longer a gap.

**Selection and formatting: both shells have them now.** This used to be a *both-shells* gap
blocked on the model carrying only a run's style name. That blocker is gone
(`App::set_char_style`/`char_style`, `doc/text-core.md`) and both shells closed it the same way:
an anchor alongside the caret (Shift+arrow, Shift+click and dragging all extend it; typing or
Enter over one erases it first, the way every other editor's Shift key does), a toolbar of
toggles that read the selection's common formatting and write through `set_char_style`, and
drawing each run as the document formatted it rather than as one plain string. `grind-text-gtk`
does it with a Pango attribute list per line (`run_attributes`); `grind-web` cuts each line into
`<span>`s at every boundary the formatting, the selection and the caret introduce
(`ui_web/src/text/runs.rs`). Both give `Title` and `Subtitle` their own faces, so a document has
one shape in both windows.

**`grind-text-gtk` only.** No `grind-ui` crate: `doc/suite.md` says to extract the shared GTK
plumbing "on evidence, at S9, when the second shell shows the seam", and one *minimal* shell
is not that evidence — this one copied the observer bridge, the `--render-to` harness and the
window-close latch, which is three data points and the right time to look again is when either
shell grows. No `.desktop` file, AppStream metainfo or icon, and no `[package.metadata.deb]`
block: packaging is S11, which does all five packages at once. No shortcuts window. No a11y
beyond the floor (`Accessible::announce` on every caret move, as M9 requires). The document
is re-laid-out in full whenever it or the width changes, so a very long document costs a pass
per resize (`ponytail` in `view.rs`).

**`grind-web` only.** Its own gap list has moved to **`doc/web-shell.md`**, which is where that
shell's answers live now — it grew a command palette, formatting, a clipboard and charts, and
"the same list, minimally" stopped being true. In short: no input method, the whole document is
in the DOM rather than windowed, and character advances are cached per character so kerning
between two of them is lost. The outline dialog and the go-to-address field are no longer gaps
— both are in the palette Ctrl+K opens, which is that shell's answer to a dialog.

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
