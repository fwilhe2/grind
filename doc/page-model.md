<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# A page model for the word processor — the plan

> **Status: DECIDED, being built** (2026-11-14). §7's seven decisions are taken as recommended.

The word processor becomes **WYSIWYG**: the editor shows pages, and the text on them is set by
the engine that writes the PDF, in the same fonts, at the same positions. Three things are asked
for, and they are one piece of work:

1. **A page model**: editing happens in pages.
2. **Nice typesetting**: kerning, word spacing, hyphenation, glyph fallback.
3. **Screen = paper**: what the window shows is what the PDF and the printer get.

This plan overturns written decisions, so it names them first:

| Written today | Where | What this plan does to it |
|---|---|---|
| "A paginated *editing* view" gated on a week of use | `doc/not-doing.md` §2, `doc/pdf-export.md` §1 and §5, `doc/text-layout.md` "What stays gated" | **Reversed.** The page view is the editing view of every graphical shell |
| "The accepted cost: the screen and the paper can break lines differently" | `doc/pdf-export.md` §1 | **Removed.** One engine sets both |
| Font metrics are the shell's, behind `Metrics` | `doc/text-layout.md` Path C | **Narrowed.** For a text document every graphical shell answers with `grind_print::Typesetter`. The trait stays: the terminal, the CLI's `--width` and the spreadsheet's cells still use their own |
| "No screen draws an aligned line" | `print/src/align.rs` | **Reversed.** Alignment moves into `grind_core::layout`, as that file says it must |
| Page geometry "read, never written" | `doc/pdf-export.md` §1 | **Narrowed** in PM8: page setup and a manual page break are authored |

What does **not** change: pages are still **derived, never stored** (`doc/view-modes.md` §1).
The "model" is a layout the core keeps current while a document is open; no page reaches a file.

---

## 1. Where things stand

Built, and reused as it is:

* `grind_text::page`: the paginator. Orphans, widows, keep-with-next, hard breaks, split table
  rows, master pages anywhere, left and right pages.
* `grind_print`: fonts (bundled Liberation, metric-compatible twins, system fonts), `Typesetter`
  (harfrust shaping, so kerning and ligatures), the display list (`ops.rs`), the PDF (krilla), the
  raster (tiny-skia), headers, footers and page numbers.
* The preview in every shell is already `raster::render` of the PDF's display list.

What stands between that and WYSIWYG, found by reading the code:

| Gap | Where |
|---|---|
| Each shell measures with its own engine (Pango, canvas, GDI, CoreText), so its line breaks are its own | `ui_*/src/metrics.rs` |
| Screens draw `look.rs`'s role faces; paper draws the document's resolved paragraph styles | `doc/pdf-export.md` P5, "Not yet: the screens" |
| Alignment and justification exist only on paper, so no caret agrees with a justified line | `print/src/align.rs` |
| The breaker measures a fragment once, and `text.rs` shapes each drawn piece again; kerning at a piece's edge can make the ink differ from the measured x | `print/src/text.rs` (`setter.shape` in `lines`) |
| `typeset` sets the whole document on every call | `print/src/lib.rs` |
| A character no face has is counted, never drawn from another face | `doc/pdf-export.md` P3 |
| No hyphenation; justification widens spaces without limit; `fo:letter-spacing` and `style:letter-kerning` are not read | — |
| No geometry going back: nothing answers "which caret is at this point of page 3" | — |

---

## 2. The design

### 2.1 One engine, in points

A text document is laid out **once**, by `grind_print`, in points, at the document's own page
geometry. Zoom and device resolution are a scale applied when drawing and never reach layout: a
line breaks in the same place at 75 % and at 400 %, on a 96 dpi panel and on a 600 dpi printer.
Advances stay unhinted and fractional for that reason.

### 2.2 How a shell draws a page (decision 1)

**Recommended: every graphical shell blits `raster::render`'s pixels**, as each preview already
does. The screen is then the PDF's display list by construction, there is one glyph renderer to
make good instead of five, and `--render-to` frames agree across shells.

The alternative is drawing the display list's glyph ids through each toolkit (Cairo, CoreText,
GDI, canvas). Positions would still agree; the ink would be each platform's. GDI snaps glyphs
to whole pixels and a canvas has no glyph-id call at all, so two of four shells would need a
workaround. It stays possible later, per shell, behind a frame comparison with the raster.

What the raster costs: grey antialiasing without hinting or LCD subpixels, so body text at
96 dpi is softer than Pango's. PM4 works on that and PM0 shows it before anything is built.

**The caret, the selection, squiggles, find hits and bookmark marks are not in the raster.**
The shell draws them over the page from geometry the core hands it, so a blinking caret
repaints nothing.

### 2.3 Where each piece lives

```
grind-core::layout   + alignment and justification in Layout, letter spacing,
                       hyphenation points handed in (no dictionary here)
grind-text::page     the paginator, as today
grind-print::set     NEW: the document kept typeset. A cache per block, re-set from the
                       first changed block, pages reported as damaged
grind-print::place   NEW: every placed line as a box: page, block, character range,
                       origin, baseline, caret xs. locate / hit / selection rectangles
grind-print::raster  + glyph cache, subpixel positions, page tiles
shells               a strip of page tiles, overlays, input forwarded. No layout
```

The font stack stays out of `grind-core` (`doc/text-layout.md` decision 2). The graphical text
shells take `grind-print` unconditionally; the CLI's `--no-default-features` build and the
terminal stay font-free.

Caret **motion** stays `App::caret_line` and `caret_line_bounds` over `Faces`; the shell passes
`grind_print`'s faces, so Down-arrow walks the paper's lines. Caret **geometry** is the new line
boxes. Both read the same `Layout`, which is why alignment has to move into the core first.

### 2.4 What cannot be WYSIWYG

* **The terminal.** Cells are not points. It keeps its continuous column and gains page
  boundaries and "page n of N" from the real paginator.
* **Across machines.** A document naming a font one machine has and another lacks is set in two
  faces. Screen equals paper on *each* machine; the report's substitution sentence is shown on
  open, not only on export. `--bundled-fonts` is the reproducible mode, as today.
* **The browser** has only the bundled faces, so it agrees with a desktop only for documents
  set in them or their metric-compatible twins.
* **The spreadsheet.** Out of scope; its cells keep the shell's metrics.

---

## 3. Milestones

### Where it stands (2026-11-14)

| | State |
|---|---|
| PM0 | **Done**: §6 |
| PM1 | **Done**: `grind_core::layout::wrap_set` aligns and justifies (`Setting`, `Align`, `Line::shift` and `stretch`), `grind-print` draws from it, and a caret motion through `Faces` sees the paragraph's indent, tabs and alignment. Item 3 turned out to hold already: measured over the Calc Guide's 564 pages, no drawn piece ends more than 0.005 pt from where the layout put its last caret — a piece is cut at run and line boundaries, where shaping it alone and shaping its run whole agree. Its test waits for PM2's line boxes, which carry the offsets it needs. Item 4 stays as `doc/text-layout.md` cost 1 names it |
| PM2 | **Done**: `grind_print::place` (`LineBox`, `locate`, `hit`, `selection`) over the line boxes typesetting leaves on each page; `grind text locate` and `hit`. A motion by a screenful is `App::caret_line` with a count, as it was, and a page's first caret is `Page::start`. A picture's caption has no carets of its own yet (`ponytail:` in `print/src/text.rs`) |
| PM3 | **Half done.** What is kept: every advance a `Typesetter` has measured, and every layout an `App` has made (keyed by the text, how its runs are set, the measure, the `Setting` and `Metrics::identity`). Setting the Calc Guide went from 5.6 s to 1.5 s, and setting it *again* — what an edit costs — takes 0.9 s, 1.6 ms a page, with byte-identical PDFs. That is a frame for a document of ten pages and not for one of five hundred. Item 1 is `grind_print::Session`: it holds the pages, sets them again whole when told of a change, and a page an edit did not touch compares equal to what it was, which is the damage a window needs. Item 2 since: shaping is kept by face, size and text; a paragraph style is resolved once per name until the document changes; pagination lays out one flow with each block at its run's measure, not one flow per measure; the whole document is read twice per setting, not seven times. Setting the Calc Guide again is now 0.38 s (0.97 s before), every step checked byte-identical on 1571 documents' PDFs — about 140 ms drawing, 140 ms pagination, 100 ms the per-block maps (spacing, labels, frames, indents) and 35 ms the page kinds. No cache left takes that under a frame. **Not yet** (items 3, 4, 6): re-cutting from the first changed block, and the equivalence test that must come with it. What makes it large: `print/src/text.rs::typeset` builds its per-block maps over the whole document before it cuts, and an Enter shifts the index every later page's line boxes and marks carry |
| PM4 | **Half done.** Items 1, 2 and 4: `raster::Rasteriser` keeps each glyph's coverage per face, pixel size and quarter-pixel phase (bounded), and each picture as drawn; the window keeps each page's raster per page and scale and drops those out of view. A Calc Guide page drawn again costs 1.5 ms at 96 dpi (17.6 before), the pictured cover 7.9 ms (101). **Not yet**: item 3 (text contrast, judged on a frame), item 5 (paper and ink colours — white paper is decision 5, so only if a dark page is ever wanted), the 60 Hz scrolling measurement |
| PM5 | **Under way.** Built: caret blink, the input method's candidate window placed from `locate` and its preedit drawn at the caret (over the text, a `ponytail:`), Fit Page, `--render-to --page n`. The window draws the session's pages on a desk (`ui_text_gtk/src/strip.rs`), white in either theme; caret, selection, spelling and grammar underlines and bookmark names over them from `place`; click, drag, double and triple click through `hit`; Up/Down and Home/End through `step` and `line_bounds`; scrolling follows the caret across pages, both ways; zoom by Ctrl+wheel about the pointer, a pinch, Ctrl+plus/minus about the caret, Fit Width and Actual Size; "Page 3 of 12" on the status line. Pango text, `geom.rs` and `metrics.rs` are gone from the shell. Ctrl+0 stays Paragraph, so 100% is on the zoom button and has no key. Every find hit is marked; the pointer is an I-beam on paper, a hand on a link. The exit holds as a widget case: the first page drawn by the window equals the preview of it, every pixel. **Not yet**: the preedit set in the line, table column drag and picture handles, character extents for accessibility (gtk4-rs has `AccessibleTextImpl::extents` only behind `v4_16`, and both GTK shells pin `v4_14`) |
| PM6 | **Item 4 done**: a soft hyphen is a break that shows a hyphen only where the line breaks — `grind_core::layout` fits the line with the hyphen's width and records it (`Line::hyphen`), alignment counts it, `grind-print` draws it as decoration so a copied word is whole. Only the two LibreOffice test documents with soft hyphens print differently. Neither confirms Writer's placement: Writer breaks their lines elsewhere for reasons that predate this (`tdf105417.odt` sets its first line shorter), so a line ending in a soft hyphen has not yet been compared with the oracle's. The other shells break there and do not draw the hyphen until PM7 draws them from the raster. **Item 1 done** (decided 2026-11-14: a curated list, checked by coverage; colour emoji in a monochrome face): `fonts::FALLBACK` names the families looked in, in order; `fonts_for` loads one only when the document uses a character no loaded face has, and only if it has one of them. The shaper sets each run of `.notdef` clusters again in the face `Fonts::fallback_for` finds, marking those glyphs with their face (`Glyph::face`); the raster draws from it and the PDF writes a run per face. Of 1571 documents, the 34 whose PDFs changed all had missing glyphs before: about 2,700 boxes became 3. A variable face is drawn at its default instance, Thin for Noto Sans CJK (a `ponytail:`). The browser has no system faces and reports as before. **Items 2 and 3 done**, from item 0's facts 51 and 52 (`doc/odt-format.md` §5c, measured on 26.8): `style:letter-kerning` and `fo:letter-spacing` are read from a run and a paragraph style into `TextStyle`; the shaper kerns unless `false` says not to, and adds the spacing after every character, on top of the kerning. Liberation Serif forms no ligatures in Writer at all, so "ligatures off when spaced" did not arise. Every word of the measuring document lands within 0.1 pt of Writer's. The run properties are carried by their style and never written from the model. **Item 6 done** from facts 53 and 54: a no-break space is not stretched (it already was not); `fo:text-align-last` sets a justified paragraph's last line (`Setting::last`); `style:justify-single-word` spreads the letters of a line with no space to widen (`Setting::single_word`, `Line::letters`); and the default tab interval is 1.25 cm where no style states one — a measuring document's words land within 0.3 pt of Writer's. Items 0 (hyphenation's counts), 5 and 7–9 open |
| PM7–PM9 | Open |

Each ends green on its own. A capability gets its line in `examples/sample-text.sh`, its row in
`doc/cli-parity-text.md` and `doc/feature-matrix.md`.

### PM0 — Decisions and measurements *(small)*

1. Measure, and write the numbers into §6: `typeset` of a long document, `raster::render` of
   one page at 1× and 2× (`print/examples/timing.rs`).
2. A side-by-side frame: a body paragraph at 96 dpi through `raster.rs` and through Pango, at
   10, 11 and 12 pt. This is the evidence for decision 1.
3. wasm: the size of `grind-web-print` and the fonts a text document would now wait for.
4. Amend `doc/not-doing.md`, `doc/pdf-export.md` §1/§5, `doc/text-layout.md`,
   `doc/text-shell.md` per the table at the top.

The clean-room notes are measured where they are first used, PM6 and PM8, and each milestone
names its own.

*Exit:* §6.

### PM1 — One layout *(medium)*

1. Move `align.rs` into `grind_core::layout`: a pass after `wrap` that shifts and widens the
   caret xs the `Layout` holds. `Fixed`-tested. `x_at`, `offset_at` and `step` agree with a
   centred and a justified line.
2. `Faces` gains the block's alignment; `print/src/text.rs` draws from the aligned `Layout`
   and stops doing its own arithmetic.
3. **Measure once, draw the same.** The breaker's shaped glyphs are kept and the drawn pieces
   cut from them, never shaped again. An invariant test over every vendored document: each
   drawn glyph run starts at its line's `x_at` and ends at the next piece's.
4. Kerning across a style boundary inside a word (a bold syllable): shape the word whole where
   the face and size are the same, and name the remaining case.
5. The PDF of every vendored document is unchanged, or each difference is explained by item 3.

*Exit:* `text/tests/pdf.rs` green; the invariant holds; caret tests on justified text.

### PM2 — Geometry both ways *(medium)*

1. `grind_print::place`: `typeset` records a `LineBox` per placed line beside the ops,
   including lines in table cells, list labels' indents and repeated heading rows (marked, not
   hit-testable).
2. `locate(Caret) -> (page, x, top, height)`; `hit(page, x, y) -> Caret`, with the rules for a
   point in a margin, between pages, in a header, below the last line.
3. `selection(from, to) -> Vec<(page, Rect)>`; `span(block, range)` for squiggles, find hits
   and link areas.
4. Page motion: Page Up and Page Down by window height, Ctrl+Page by page, "go to page n".
   A caret at a page boundary belongs to the page whose line holds it.
5. CLI (rule 4): `grind text locate report.fodt p12+40`, `grind text hit report.fodt --page 3
   --at 120,300`, `grind text preview --caret` draws the overlay. Parity rows.
6. Property test: `hit(locate(c)) == c` for every caret of every vendored document.

*Exit:* the property test; the CLI verbs in `sample-text.sh`.

### PM3 — The document kept typeset *(large)*

1. `grind_print::set::Session`: owns the `Typesetter`, subscribes to the `App` (rule 3), holds
   pages and line boxes for the current revision.
2. A cache per block keyed by what its layout depends on: its content revision, measure,
   resolved style. Shaping results cached per (face, size, text) with a bound.
3. Re-set from the first changed block; stop where a page starts on the same line as before,
   and carry the rest over. Page count, header and footer fields (`text:page-count`) are redone
   only when the count changes.
4. Damage: the session reports which pages changed, so a shell repaints those.
5. A change to styles, page geometry or fonts drops the whole cache; named as the slow path.
6. Equivalence test: after every edit of a scripted session, the incremental result equals a
   fresh `typeset`. Fuzzed over the vendored documents.
7. Budget, from PM0's numbers: a keystroke in a 500-page document re-sets and repaints inside
   one frame at 60 Hz. If the first cut misses it, set on a worker and say so with a
   `ponytail:`.

*Exit:* the equivalence test; the budget met or its ceiling written down.

### PM4 — The raster, good enough to read all day *(medium)*

1. A glyph mask cache per (face, glyph, size, quarter-pixel offset), bounded.
2. Fractional glyph positions, so kerning survives at small sizes.
3. Text contrast: gamma-correct blending, and stem darkening if PM0's frames show thin text.
4. Tiles: a page rendered at the zoom's scale, kept per (page, revision, scale), repainted on
   damage, dropped off screen.
5. `raster::render` takes paper and default-ink colours (decision 5).
6. The `hayro` cross-check still passes: our raster of a page against a raster of our PDF.

*Exit:* PM0's frame redone and judged; scrolling the Calc Guide holds 60 Hz in GTK.

### PM5 — The GNOME window, the reference shell *(large)*

1. `view.rs` draws a strip of pages on a desk: tiles as `gdk::MemoryTexture`, a gap and a
   shadow between pages, facing pages not offered.
2. Zoom: Ctrl+wheel, Ctrl+0, fit width, fit page; kept per window, never written to the
   document.
3. Overlays from PM2: caret (blink), selection, spelling and grammar underlines, find hits,
   bookmark marks, link hover and Ctrl+click.
4. Pointer: click, Shift+click, drag, double and triple click, through `hit`.
5. Input method: the preedit is set by the session as a pending fragment
   (`App::layout_composing`'s job), and the candidate window is placed from `locate`.
6. Scrolling follows the caret across a page gap; the scrollbar is sized from page count.
7. Status: "Page 3 of 12" beside the address; the font substitution sentence as a toast on
   open.
8. Tables, pictures, lists and captions come from the display list, so their drawing code in
   the view is deleted. Table column drag and picture selection handles go through `hit`.
9. Accessibility: the text interface answers character extents from `locate`.
10. `--render-to` is a page-view frame; `--render-to --page n`. New baselines, committed with
    the reason.
11. The continuous view, the Pango `Metrics` for text and `look.rs`'s screen faces are removed
    from this shell (decision 2).

*Exit:* a screenshot of page n in the window equals `grind text preview --page n` at the same
scale, overlays off, byte for byte.

### PM6 — Typesetting *(large; items 1–4 before PM7)*

1. **Glyph fallback**: a character the face lacks is drawn from another that has it, chosen
   by coverage from the system's fonts. Per cluster, in the shaper, so measure and draw agree.
   The browser counts and reports, as today. Typing an emoji must not show `.notdef`.
0. Notes first, each `MEASURED` on the pinned image or `SPEC` with an rng line, in
   `doc/odt-format.md` §5c: what Writer writes for `style:letter-kerning` (rng:13705) and
   `fo:letter-spacing` (rng:13532) by default; `fo:hyphenate` and its counts (rng:13773–13783,
   rng:12533, rng:12541); how a justified line treats a no-break space, a tab and a line ending
   in a manual break; `fo:text-align-last` and `style:justify-single-word` (rng:12490, 12499).
2. **Kerning as the document says**: `style:letter-kerning`, on unless a style turns it off.
3. **Letter spacing**: `fo:letter-spacing`, added after shaping, ligatures off when non-zero.
4. **Soft hyphen** (U+00AD): a break point that draws a hyphen only where the line breaks.
5. **Hyphenation**: `fo:hyphenate` with its remain, push and ladder counts, by the paragraph's
   `fo:language`. Dictionaries: check first whether the Hunspell projects `grind-spell` already
   vendors ship `hyph_*.dic` under a licence we can carry, before taking a TeX-pattern crate.
   Licences read from the files. The core breaker takes hyphenation points through a trait;
   `grind-print` (or `grind-spell`) answers it.
6. **Justification**: the last line and a line ending in a manual break stay ragged; a
   single-word line is not stretched (`style:justify-single-word`); `fo:text-align-last`;
   spaces after the last tab only. Each from item 0's notes.
7. **Line breaking** (decision 4): greedy, as Writer breaks, so a document has the page count
   Writer gives it.
8. `style:text-position` (super- and subscript) and `fo:font-variant="small-caps"` drawn if
   the audit finds them undrawn.
9. Samples: `examples/sample-text.sh` gains a justified, hyphenated German paragraph and a
   kerning pair line (`AVATAR`, `To`, `fi`).

*Exit:* each property has a `Fixed` test for its arithmetic and a raster test for its ink;
loop C still green.

### PM7 — The other shells *(large)*

Each: page strip, zoom, overlays, pointer, IME, status, its own `Metrics`-for-text removed, its
doc's gap list and feature-matrix rows updated.

1. **Browser**: `grind-web-print` loads when the first text document opens, fonts fetched from
   the page's origin then (decision 3). Pages on `<canvas>`, overlays as positioned elements,
   the hidden input for IME placed from `locate`. `smoke.js` checks a page count and a hit.
2. **Windows**: tiles through `StretchDIBits`, as `--render-to` does; the GDI text path stays
   for chrome and the grid. Checked under Wine with a screenshot.
3. **macOS**: tiles as `CGImage` in `page_view.rs`; `NSTextInputClient` ranges answered from
   `locate`. Type-checked here, run through `mac-drive.yml`.
4. **Terminal**: page boundaries as a rule in the column and `:page n`, when built with `pdf`.
5. Every graphical shell's frame of page n equals the CLI's, at the same scale.

### PM8 — Authoring pages *(medium)*

1. **Page break**: Ctrl+Enter writes what Writer is measured to write (`fo:break-before`,
   rng:1297, and on which style; a note in `doc/odt-format.md` §5c first); `grind text break`.
   Removing one is Backspace at the top of the page it opened.
2. **Page setup**: paper (ISO 216 names, `doc/pdf-export.md` decision 7), orientation, four
   margins, for the document's default master page. Written to `style:page-layout`, changing
   only those attributes (R6); `grind text page-setup`. `never_worse.rs` and loop C cover it.
3. Projection: page setup stops being a named gap in `doc/projection-text.md`, or stays one
   with the reason.
4. Not here, each named in `doc/not-doing.md` §2: editing a header or footer in place,
   footnote placement, columns.

### PM9 — Proof *(large, can start after PM1)*

1. **Screen = PDF**: for the sample and every vendored document, the tile of each page against
   `hayro`'s raster of our PDF, within the existing tolerance.
2. **Print at actual size**: GTK's print dialog, PDFKit and the Windows raster path are checked
   not to scale to fit; a test page with a 100 mm rule.
3. **Loop G** (`doc/pdf-export.md` P9, still open): page count, first line of each page and
   line breaks per paragraph against LibreOffice's PDF on the pinned image, with a `FLOOR`.
4. `CLAUDE.md`'s summary of `grind text` and the loop table updated.

---

## 4. Order and dependencies

```
PM0 ─ PM1 ─ PM2 ─ PM3 ─ PM4 ─ PM5 ─ PM6(1–4) ─ PM7 ─ PM8
        └──────────────── PM9.3 (loop G) ─────────────┘   PM6(5–9) any time after PM1
```

PM1–PM4 change no window and are testable from the CLI alone. PM5 is the first thing to look
at, and the point to stop and judge before three more shells follow.

## 5. Risks

1. **Soft text at 96 dpi** (decision 1). Seen in PM0, worked on in PM4; the fallback is native
   glyph drawing in the shell that needs it.
2. **Incremental layout is where the bugs will be**: a stale cache shows the wrong text. The
   equivalence fuzz in PM3 is the guard, and it must exist before any shell uses the session.
3. **Speed on long documents.** Measured before designed (PM0).
4. **Hyphenation dictionaries' licences.** Checked before code; soft hyphens work regardless.
5. **Browser weight**: 2.6 MB gzipped of module plus the fonts (§6), for text documents only.
6. **Every text `--render-to` baseline changes** in PM5 and PM7. One commit per shell, saying so.

## 6. Measurements

2026-11-14, a release build on the development machine (`print/examples/timing.rs`), over
LibreOffice's Calc Guide: 564 pages as we set it, about 9400 blocks.

| | |
|---|---|
| Open | 0.4 s |
| Typeset, the whole document | 5.6 s — **10 ms a page** |
| Typeset, after PM3's caches | 1.5 s, and 0.9 s set a second time |
| Raster, one page of body text, 96 dpi (793 × 1122) | 22 ms |
| The same at 192 dpi (1587 × 2245) | 39 ms |

What they decide:

* **Setting the whole document per keystroke is out by a factor of several hundred**, so PM3 is
  not optional, and a long document's first layout wants to show its first pages before the
  last are set.
* **A page drawn from scratch misses a 16 ms frame**, at either scale. PM4's glyph cache has to
  bring a whole page under it, or a keystroke repaints only the band of lines that changed.
* **The raster reads well at 96 dpi.** The same sentence in Liberation Serif at 10, 11 and
  12 pt, through `raster.rs` and through `pango-view`: ours is a little lighter and evenly
  spaced; Pango's hinted advances open and close gaps between letters ("over the", "fi fl").
  Decision 1 stands on that frame, and stem darkening (PM4 item 3) is optional.
* **The browser's cost is larger than first written**: `grind_web_print_bg.wasm` is 8.9 MB,
  2.6 MB gzipped (it carries its own copy of the text core), and the twelve Liberation faces
  are 2.4 MB gzipped together. PM7 loads the faces a document names, not all twelve, and looks
  at folding the two modules into one.

## 7. The decisions

All seven taken as recommended, 2026-11-14.

1. **Shells blit the shared raster**, rather than drawing glyphs natively. Recommended.
2. **The page view replaces the continuous view** in the four graphical shells; the terminal
   keeps its column. Recommended: two views are two layouts, and one of them would not be what
   prints.
3. **The browser loads the print module and fonts when a text document opens.** Recommended;
   there is no WYSIWYG there otherwise.
4. **Greedy line breaking with hyphenation**, matching Writer's breaks, rather than
   paragraph-optimal (Knuth–Plass) breaking. Recommended. Optimal breaking gives more even
   word spacing, but a document would paginate differently here than in Writer, loop G could
   not hold, and lines above the caret would move while typing.
5. **Paper is white in a dark theme**, with the desk around it themed. Recommended; a dark
   paper is a later view mode over the raster's two colours.
6. **Authoring is page break and page setup** (PM8); header and footer editing, and footnotes,
   follow separately. Recommended.
7. **WYSIWYG is per machine**, with substitutions said on open. There is no alternative short
   of refusing system fonts.

## Sources

| | Where |
|---|---|
| Line layout in the core, metrics injected | `doc/text-layout.md` |
| Pagination, fonts, the display list, the preview | `doc/pdf-export.md` |
| Derived, never stored | `doc/view-modes.md` §1 |
| One engine measures and draws | `doc/windows-shell.md`, W5a |
| The gates this reverses | `doc/not-doing.md` §2 |
| `fo:letter-spacing`, `style:letter-kerning` | rng:13532, rng:13705 |
| `fo:hyphenate`, remain and push counts, keep, ladder | rng:13773, rng:13778, rng:13783, rng:12533, rng:12541 |
| `fo:text-align-last`, `style:justify-single-word` | rng:12490, rng:12499 |
| `style:text-position`, `fo:font-variant` | rng:13379, rng:13316 |
| `fo:break-before` | rng:1297 |
