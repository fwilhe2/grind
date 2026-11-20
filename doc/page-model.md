<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# A page model for the word processor

> **Status: built** (decided 2026-11-14). §3 is what is not.

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
| Page geometry "read, never written" | `doc/pdf-export.md` §1 | **Narrowed**: page setup and a manual page break are authored |

What does **not** change: pages are still **derived, never stored** (`doc/view-modes.md` §1).
The "model" is a layout the core keeps current while a document is open; no page reaches a file.

---

## 1. What it is built on

* `grind_text::page`: the paginator. Orphans, widows, keep-with-next, hard breaks, split table
  rows, master pages anywhere, left and right pages.
* `grind_print`: fonts (bundled Liberation, metric-compatible twins, system fonts, a curated
  fallback list), `Typesetter` (harfrust shaping, so kerning and ligatures; hyphenation by
  Liang's patterns), the display list (`ops.rs`), the PDF (krilla), the raster (tiny-skia),
  headers, footers and page numbers.
* `grind_print::session`: a document kept typeset while it is open, set again from the first
  page an edit changed, and `place`, `strip` and `view`: every question a window asks about
  where its text is.

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
96 dpi is softer than Pango's; text sizes are darkened to make up for it (`raster::contrast`).

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

The font stack stays out of `grind-core` (`doc/text-layout.md` decision 2). Every shell and the CLI
take `grind-print`; the terminal draws no glyph from it, and uses it to set the pages it rules
into its column.

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

## 3. What is not built

* **Setting again from the change** covers an edit that keeps or changes a plain paragraph's
  lines; an edit to a heading, a list item added or removed, a table cell or a picture sets the
  whole document again.
* **Windows**: a find hit is not marked on the page. **The browser**: no zoom of its own (the
  browser's scales the page), no marked find hits, and no input method composing in line.
* **GTK**: no table column drag or picture handles, and no character extents for a screen
  reader (gtk4-rs has them only from GTK 4.16; the shells pin 4.14).
* **Hyphenation**: no `fo:hyphenation-ladder-count`, no run's own language, no compound-word
  levels; patterns for English and German only.
* **Proof**: printing at actual size has not been checked on paper, and loop G waits for the
  pinned LibreOffice image to carry the bundled faces (`doc/odt-format.md` §5c fact 7).

## 4. Measurements

2026-11-14, a release build on the development machine (`print/examples/timing.rs`), over
LibreOffice's Calc Guide: 564 pages as we set it, about 9400 blocks.

| | |
|---|---|
| Open | 0.4 s |
| Typeset, the whole document | 5.6 s — **10 ms a page** |
| Typeset, with the session's caches | 1.5 s, and 0.9 s set a second time |
| Raster, one page of body text, 96 dpi (793 × 1122) | 22 ms |
| The same at 192 dpi (1587 × 2245) | 39 ms |

What they decide:

* **Setting the whole document per keystroke is out by a factor of several hundred**, so the
  session sets again only what an edit changed, and a long document's first layout wants to
  show its first pages before the last are set.
* **A page drawn from scratch misses a 16 ms frame**, at either scale. The glyph cache has to
  bring a whole page under it, or a keystroke repaints only the band of lines that changed.
* **The raster reads well at 96 dpi.** The same sentence in Liberation Serif at 10, 11 and
  12 pt, through `raster.rs` and through `pango-view`: ours is a little lighter and evenly
  spaced; Pango's hinted advances open and close gaps between letters ("over the", "fi fl").
  Decision 1 stands on that frame, and stem darkening is optional.
* **The browser's cost is larger than first written**: `grind_web_print_bg.wasm` is 8.9 MB,
  2.6 MB gzipped (it carries its own copy of the text core), and the twelve Liberation faces
  are 2.4 MB gzipped together. The browser fetches all twelve with the first text document.

## 5. The decisions

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
6. **Authoring is page break and page setup**; header and footer editing, and footnotes,
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
