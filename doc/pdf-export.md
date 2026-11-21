<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# PDF export for the word processor — the plan

> **Status: built** (decided 2026-10-04); §6 is what is not. **Paper is ISO 216, never a US size** (§8, decision 7). The spreadsheet is out of scope on purpose: printing a
> spreadsheet is rarer, and its page model (print ranges, repeated rows, scaling to fit) is a
> different program. Nothing below shuts that door. The generic half is kept generic so that
> `grind sheet export-pdf` could reuse it later.

This plan overturns two written decisions, so it says which ones first:

| Written today | Where | What this plan does to it |
|---|---|---|
| "**Print to PDF** — via the platform, so it needs a platform" | `doc/not-doing.md` §2 | **Reversed.** The core writes the PDF, and a platform only *prints* one (§3) |
| "Pagination — gated, loop D at a stated floor, and a week of use proving the continuous view insufficient" | `doc/not-doing.md` §2, `doc/text-layout.md` (in git) | **Split in two.** Pagination *as an output* (PDF, preview) comes in, with its own differential as the gate it always named. A paginated *editing view* stays gated, on the same condition as before |
| "Loop D" as pagination's differential | `doc/not-doing.md`, `doc/text-layout.md` | **Renamed loop G.** The xlsx filter has used loop D since phase 11 |
| Named paragraph styles "kept as a name, never resolved" | `doc/text-core.md`, Styles | **Narrowed**. They are *resolved for layout* and still never authored |

---

## 1. The question that decides the shape: do we need a page model?

**We need pagination. We do not need a page *model*.** These two get mixed up, and the plan
depends on keeping them apart.

* A **page model** in the stored sense means the document holds pages, and the editor edits
  inside them: page view, the caret moving across a page break, "page 3 of 12" in the status
  bar. That is Path B from `doc/text-layout.md`. Nothing about PDF export needs it.
* **Pagination** is a *pure function*: document + page geometry + fonts → pages. It runs when
  you export or preview, and the result is thrown away afterwards. The project already treats
  `doc/view-modes.md`'s roles and names this way: **derived, never stored**, because a stored
  copy goes stale and a derived one cannot. Pages are the same kind of thing.

So the answer to "can we do real layout only at print/preview time?" is **yes, and it is the
right design, not a shortcut.** This is what Writer's web view and Word's draft view already
do: the editing surface reflows, and the printed page is computed. The editor stays the
continuous column it is today, in every shell.

What *does* have to enter the model, read-only:

1. **Page geometry**: `style:page-layout` (rng:12213) through `style:master-page` (rng:12141).
   That means `fo:page-width`/`fo:page-height` (rng:12255/12260), the four margins, and
   `style:print-orientation`. Today this is kept byte for byte through R6 and never looked at.
   It becomes *read* as well, and is still never written: R6 already carries it out unchanged.
2. **The paragraph properties pagination honours**: `fo:break-before`/`fo:break-after`,
   `fo:keep-with-next` (rng:2110), `fo:widows`/`fo:orphans` (rng:12512/12517). Direct ones,
   and ones inherited through a named style.

Because both are derived, a later **print-layout view** in a shell needs no further model
change. It is the same paginator's output, drawn live. That is the natural follow-on to the
preview, and it stays gated as stated above.

> **Amended 2026-11-14 by `doc/page-model.md`**: the paragraph below was the cost accepted
> then. The editor is moving onto this engine, so that the screen and the paper are one layout.

**The accepted cost**: the screen and the paper can break lines differently, because each
shell measures with its own font engine and the PDF measures with the font file it embeds. On a
desktop where both find the same font files, the breaks will usually agree, but nothing
guarantees it. This is the same split as Writer's web view against its print view. The
**preview** is always exactly the PDF (§4), so "what will print" is always visible.

---

## 2. The library: printpdf, or something better?

Short answer: **krilla** is the better fit. Versions below are as of 2026-10-04 (`cargo search`
/ `cargo info`).

What the project needs from a PDF library is narrow, because **we have a layout engine** and
want to keep it, since it is the one the caret already agrees with:

* take positioned **glyphs** from a font file we chose, and embed that font **subset**;
* write `ToUnicode` correctly, so text copies and searches as text;
* images (PNG, JPEG passed through), filled rectangles, lines;
* outline (bookmarks), links, document metadata;
* **tagged PDF** (headings, paragraphs, lists and tables as structure) and PDF/A, because "ready
  for print" and "archivable" overlap in practice, and screen readers depend on tags;
* deterministic bytes, so a test can render twice and compare, the way `--render-to` does.

| Crate | Licence | Fit |
|---|---|---|
| **krilla 0.8** | MIT OR Apache-2.0 | **Built for exactly this** (`Surface::draw_glyphs`: glyphs, font, source text): a high-level writer for callers that already have their own layout. Typst's PDF backend (built on `pdf-writer`, from the same people). Fonts in every OpenType flavour, subset with `subsetter`. Colour glyphs fall back to Type 3. `ToUnicode` handled. Tagged PDF / PDF/UA, PDF/A-1..4 with **built-in validation** (it refuses to write a non-conforming file instead of writing one silently). CMYK and ICC colour exist (`graphics/color.rs`), but **PDF/X does not**: its PDF/A-4 notes say PDF/X and PDF/E are unsupported. Images. Default features pull `rustybuzz` (`simple-text`) and image decoders, and both can be turned off. MSRV 1.92, toolchain here is 1.98 |
| printpdf 0.12 | MIT | Usable, and its "print-friendly" claim is real but means **prepress**: it can label a file PDF/X-1a/3/4/5 and attach an ICC output intent and XMP (`conformance.rs`). It does not check that the content conforms; the flag only switches on the ICC profile and the XMP (`serialize.rs:119,151`). The deciding fact is **font subsetting only exists behind its `text_layout` feature, which pulls in `azul-layout`, `azul-core` and `rust-fontconfig`**. Without it, `subset_font` embeds the whole font file (`font.rs:968`). So a caller with its own layout gets either a second layout engine in the tree or every PDF carrying full fonts. Its subsetter is a fork (`allsorts-azul`). No structure tree, so no tagged PDF. Its versions went 0.8 → 0.12 in about a year |
| pdf-writer 0.15 | MIT OR Apache-2.0 | The layer under krilla. Zero dependencies, very fast, but **we** would write CID fonts, `ToUnicode` CMaps, subsetting and the structure tree. That is krilla's whole job, done again |
| lopdf | MIT | Reads and edits PDFs. The wrong side of the problem |
| genpdf | — | Layout on top of an old printpdf, unmaintained |

The rest of the stack, all format-neutral plumbing in this project's sense (CLAUDE.md: "can be
lazy; semantics never are"):

| Job | Crate | Licence |
|---|---|---|
| Find system fonts, match by family/weight/style | `fontdb` 0.24 | MIT |
| Read metrics, ascent/descent, glyph outlines | `skrifa` **0.42**, the version krilla 0.8.2 uses (0.48 is current) | MIT OR Apache-2.0 |
| Shaping: kerning, ligatures, combining marks | `harfrust` **~0.8** (HarfBuzz org's port, the successor to `rustybuzz`). 0.14 is current, but anything past 0.8 pulls in a second `read-fonts`/`skrifa` beside krilla's. Measured with `cargo tree -d` | MIT |
| Rasterising the preview | `tiny-skia` 0.12 | BSD-3-Clause |
| A *second* opinion on our PDF, tests only | `hayro` 0.8 (pure-Rust PDF rasteriser, same authors as krilla) | Apache-2.0 OR MIT |

**Recommendation: krilla, `default-features = false`, pinned to an exact version** (it is pre-1.0),
with `harfrust` doing our own shaping, so that measuring and drawing go through **one engine**.
That is the rule the Windows shell learned from bugs: measure with one engine and draw
with another, and the caret lands where the ink is not. Here the "caret" is every glyph
position on the page.

Every licence above was checked **in each crate's own `Cargo.toml`, not from memory**, the way
`doc/text-layout.md` decision 2 did for `unicode-linebreak`. It also runs `cargo tree -d` on
the result: krilla and harfrust must agree on `read-fonts`/`skrifa`, or we drop krilla's
`simple-text` so that only one shaper is in the binary.

---

## 3. Architecture

```
            grind-core                     grind-text                        grind-print (new)
  ┌──────────────────────────┐  ┌────────────────────────────┐  ┌───────────────────────────────────┐
  │ layout.rs   (+baseline,  │  │ page.rs  paginate(): pure, │  │ fonts/   fontdb + skrifa + harfrust│
  │              +alignment) │◄─┤   Fixed-testable, lines →  │◄─┤          → impl Metrics (points)   │
  │ page.rs     PageGeometry │  │   pages, keep/widow/orphan │  │ ops.rs   a page as a display list  │
  │   (ODF lengths, generic) │  │ flow.rs  (table arithmetic │  │ pdf.rs   ops → krilla → bytes      │
  └──────────────────────────┘  │   reused)                  │  │ raster.rs ops → tiny-skia → RGBA   │
                                │ odf/read  page-layout,     │  │ report.rs what was substituted     │
                                │   paragraph props (read)   │  └───────────────────────────────────┘
                                └────────────────────────────┘        ▲          ▲            ▲
                                                                  grind-cli   the shells   (tests: hayro)
```

### Where each piece lives, and why

* **`grind_core::page`: page geometry.** `PageGeometry { width, height, margins }` in points,
  parsed from ODF lengths. It is generic (the `style:` namespace, and a spreadsheet has a page
  layout too), so it is R8-clean. No pagination happens here.
* **`grind_core::layout`: two additions every shell benefits from**, made in the core so the
  caret agrees with them:
  * a **baseline** per line: `Metrics` gains `ascent(style)` with a default, and `Line` gains
    `baseline`. A PDF places glyphs on a baseline, and on a line with mixed sizes that is the
    largest ascent, not `height × 0.8`. The shells draw through toolkits that work out baselines
    themselves, so for them this is unused until it is useful.
  * **alignment** (start/centre/end/justify) as a pass after `wrap`, adjusting the caret xs the
    `Layout` already holds. Justify spreads each line's slack across its spaces, except on the
    last line.
* **`grind_text::page`: the paginator.** It uses the word processor's vocabulary (blocks,
  headings that keep with what follows, table rows), so it is `grind-text`'s and not the core's,
  for the same reason `Faces` is. It is **pure arithmetic over `Layout`s**, built like
  `flow.rs`, and is testable with `Fixed` metrics and no font anywhere. That is also how the
  CLI answers "which page is `p12` on?" (rule 4). It reuses `flow.rs`'s table arithmetic but
  places **lines**, not blocks, because a paragraph splits across a page boundary and a block
  does not.
* **`grind-print`: the font stack and the two backends.** It is a separate crate because
  `doc/text-layout.md` decision 2 kept the font stack *out of `grind-core`*, and this keeps that
  promise. Internally it splits like the core does: `fonts/`, `ops.rs` and both backends are
  generic, and `text.rs` (blocks → ops) is the word processor's half. A future
  `grind sheet export-pdf` would add a `sheet.rs` beside it.

### The display list is the seam

`ops.rs` describes a page as data: positioned **glyph runs** (face id, glyph ids, advances,
the source text for `ToUnicode`), filled rects (highlight, table rules), lines (underline,
strike), images, link areas, and structure marks for tagging. `ui_mac/src/ops.rs` already
uses this pattern for its frames: decide everything portably, then execute it.

There are two executors over the same list:

* `pdf.rs` → krilla → bytes.
* `raster.rs` → tiny-skia → an RGBA buffer, using glyph outlines from skrifa. This is the
  **preview**, and also the assertable output for tests.

Because both read one list, **the preview is the PDF** by construction rather than by
agreement. A test confirms it anyway: rasterise our PDF with `hayro` and compare it with
`raster.rs`'s frame within an antialiasing tolerance. That catches a PDF backend bug that the
raster would hide.

### Fonts: the problem that actually decides quality

Most "my PDF looks wrong" reports come down to font *matching*, not to PDF bytes. The plan:

1. **Resolve** each run's `fo:font-family` (or `style:font-name` through
   `office:font-face-decls`) with weight and style against a `fontdb`:
   * on a desktop, system fonts plus the bundled set;
   * in tests and CI, and with `--fonts-only DIR`, **only** the named or bundled fonts, so the
     output is reproducible bytes.
2. **Metric-compatible substitution** when the named family is missing: Times New Roman ↔
   Liberation Serif, Arial ↔ Liberation Sans, Courier New ↔ Liberation Mono, Calibri ↔
   Carlito, Cambria ↔ Caladea. Each of these pairs is published by the substitute font itself,
   and that published claim is the citation, not anything read in LibreOffice. A substitute
   with the same metrics breaks lines in the same places.
3. **Per-glyph fallback** for characters the chosen face lacks (CJK, symbols): query `fontdb`
   by coverage. On the web, and in the bundled-only mode, a character with no glyph anywhere is
   *counted*, not drawn as tofu without anyone noticing.
4. **A report, like the xlsx filter's.** `export` returns `(bytes, Report)`, and the report
   names every substitution ("'Calibri' set in Carlito, metric-compatible"; "14 characters had
   no glyph in any font"). Every shell shows `Report::summary`, the same one-sentence pattern as
   `grind_xlsx::Report::summary`.

**Bundled fonts**: Liberation Sans/Serif/Mono in four styles. They are LibreOffice's own
defaults, so this matches the oracle and makes loop G meaningful. OFL-1.1, so REUSE needs
`LICENSES/OFL-1.1.txt` and a `REUSE.toml` entry. They cost roughly 4 MB uncompressed. That is
measured, so the `report` job will show it. Proposal: embed them in the native binaries behind
a `bundled-fonts` feature. The **browser fetches them from its own origin on first export**
instead of carrying them in the wasm, because the wasm's size is a number this project tracks.
CJK (Noto CJK, about 16 MB) is not bundled. It is system fallback only, and a named gap in the
browser.

### Printing to paper

Once the core makes the PDF, "print" means **hand the PDF to the platform's PDF printing**,
not "draw again through the platform's print API":

| Shell | Export | Print |
|---|---|---|
| `grind-text-gtk` | `gtk::FileDialog` → bytes | `GtkPrintUnixDialog` + `GtkPrintJob::set_source_file` takes a PDF directly. No cairo drawing of our own |
| `grind-web` | a download (`App::save_bytes`' path) | the PDF as a blob in a new tab; the browser's viewer prints it |
| `grind-mac` | `NSSavePanel` | `PDFDocument` → `printOperation(for:)`. PDFKit is the platform convention, and its `PDFView` can *also* be the preview there (decision below) |
| `grind-win32` | `IFileDialog` | Windows has no built-in way to print a PDF. Print the **raster** pages through `StartDoc`/`StretchDIBits` at the printer's DPI. Correct, but heavier than a vector print. Named |
| `grind-tui` | `:pdf out.pdf` | none. A terminal does not print. Named gap |

---

## 4. The preview

One rule: **the preview shows the PDF's pages, rasterised from the same display list.** It is
never re-laid-out with the shell's own fonts. If it were, the preview would be a fourth layout
that disagrees with the paper, and the preview exists to rule that out.

* **Core API**: `grind_print::preview(app, &Options, page, scale) -> (Rgba, PageInfo)` plus
  `page_count`. Pagination is cached per document revision, so flipping pages does not paginate
  again.
* **CLI** (rule 4): `grind text preview report.fodt --page 3 --dpi 96 -o p3.png`, and
  `grind text pages report.fodt` lists each page's first and last address (`p12+40`, `§2.1`).
  That list is also what loop G compares.
* **GTK**: a *Print Preview* window. A vertical strip of pages with zoom (Ctrl+wheel, as the
  sheet window has), page n/N in the header, and Export… and Print… buttons. A page is a
  `gdk::MemoryTexture` from the RGBA. Raster at the zoom's DPI, rendered lazily for the visible
  pages.
* **Web**: the same strip, each page drawn into a `<canvas>` with `putImageData`, reached from
  the palette (`doc.preview`). Alternative: the browser's own PDF viewer in an `<iframe>` over
  a blob. That is less code but gives up control, and the page/zoom chrome would differ from
  the GTK window. Recommendation: canvas, so all clients have one shape.
* **Windows**: a modal page strip blitting the RGBA with `StretchDIBits`, the same
  `CreateDIBSection` path as `--render-to`.
* **macOS**: decision 3 of `doc/macos-shell.md` says AppKit draws the chrome. A `PDFView` over
  our own bytes *is* the platform's preview, and it shows the PDF itself rather than our
  raster of it. Recommendation: use `PDFView` there. It is the one client where the native
  answer is also the truest one.
* **TUI**: page count and each page's address range in a pane (`grind text pages`' data).
  Sixel/kitty graphics are a named gap.

---

## 5. What "proper, ready for print" includes, and when

The first PDF should arrive early, and fidelity should then go up one ratchet at a time.

### In, by the end of the plan

* page size, orientation and margins from the document's own master page, and **A4 portrait with 2 cm
  margins** when it states none: ISO 216 always, never decided by locale (§8, decision 7);
* lines broken by the core breaker (UAX #14) on real font metrics, glyphs shaped with kerning
  and ligatures, every `CharStyle` property drawn (bold, italic, underline, strike, family, size,
  colour, highlight, code);
* headings, lists with their bullets (`paint::bullet`, the same marks as the screen), tabs;
* pagination: hard page breaks (`fo:break-before`/`after`), widows and orphans (the default when
  unstated is measured, not assumed), headings kept with the next paragraph, an image never split and scaled to
  the text area if larger, a table row that does not fit split between lines and each part
  drawn as a closed row, as Writer does (`doc/odt-format.md` §5c fact 25) — a heading row and a
  row stating `fo:keep-together="always"` move whole;
* tables as grids with their rules;
* images (`Run::Image`: PNG/JPEG straight into the PDF; anything else counted in the report);
* **PDF outline from the headings** (cheap, and the most-used navigation in a long PDF),
  internal links from bookmarks, metadata (`dc:title`, `dc:creator`, `meta:creation-date` from
  `meta.xml`, used *as written*, never "now", so the output is deterministic);
* **tagged PDF**: H1–H6, P, L/LI, Table/TR/TD, Figure with alt text when the frame has
  `svg:title`/`svg:desc`. Optionally `--pdfa 2b|2u|3b`, validated by krilla;
* **headers and footers with page numbers**. This needs the scope line moved:
  `text:page-number` is a field `doc/text-core.md` gated *on the page model*, and pagination now
  exists for output. Headers' and footers' *content* is ordinary blocks in the master page.

### Out, named, each with its gate

| | Why out | Gate |
|---|---|---|
| Footnotes and endnotes | **Printed since 2026-11-17** (`doc/odt-format.md` §5c fact 67): a page's footnotes at its foot under their rule, their room taken from its body, endnotes on pages after it. Still out: a note cited in a table cell, and one split across pages | a document that needs either |
| RTL, vertical writing | inherited exclusion, `doc/text-layout.md` decision 1 | that document's gate |
| Sections, columns, frames that text flows around | `doc/not-doing.md` §1, unchanged | unchanged |
| PDF/X, CMYK, bleed and crop marks (prepress) | An ODF text document is RGB with no bleed; office and home printers want a well-formed PDF with embedded fonts, which is what this plan makes. krilla has no PDF/X | somebody sending a document to a commercial printer. Then the choice is writing PDF/X ourselves on pdf-writer, or a different backend behind `pdf.rs`, which is the only file that knows the library |
| Table of contents, fields beyond page number/count | small named set later | a request |

---

## 6. What is not built

* **PDF/A.**
* **A header's or footer's table grid and frames**: their paragraphs print in order, without the
  grid.
* **Loop G**, the pagination differential: our pages against LibreOffice's PDF of the same file,
  page count, each page's first line and the line breaks per paragraph, ratcheted by a `FLOOR`.
  It waits for the pinned LibreOffice image to carry the bundled faces, since a digest pins fonts
  as well as renderer (`doc/odt-format.md` §5c fact 7).

### Tests carried throughout

* `grind_text::page` with `Fixed` metrics: every pagination rule asserted exactly, no fonts.
* `text/tests/pdf.rs`: every vendored Writer document exports without error (loop A for the
  exporter); `qpdf --check` where present (the claude-vm image should grow `qpdf` and
  `poppler-utils` for `pdftotext`, so they run there and in CI instead of skipping); krilla's
  PDF/A validation on `--pdfa`.
* Determinism: export twice, compare bytes. This is `--render-to`'s rule applied to PDFs.
* Text truth: `pdftotext` output equals the document's text, so `ToUnicode` and reading order
  are right. This is the cheapest accessibility check available.
* Never-worse stays untouched: exporting is a read, and a document exported and then saved
  must be byte-identical to one only saved.

---

## 7. Risks, in the order they would bite

1. **Style resolution scope creep.** "Resolve paragraph styles" pulls toward list styles,
   outline numbering and conditional styles. The line: the properties §5 lists and nothing
   else. Numbered lists (`text:list-level-style-number`) are their own row if a document needs
   them.
2. **Font matching versus LibreOffice.** Loop G will fail at first for reasons that are about
   fonts, not our breaker. That is why the bundled set is LibreOffice's own defaults and why
   loop G runs only with fonts the image pins.
3. **wasm size. Measured 2026-10-04**, with a scratch crate running the whole stack (harfrust shaping → krilla PDF, skrifa outlines → tiny-skia PNG) built for `wasm32-unknown-unknown` with `opt-level = "z"`, LTO: **2.0 MB raw, 704 KB gzipped**. Today's `ui_web/dist/grind_web_bg.wasm` is 4.6 MB raw and 1.33 MB gzipped, so folding the stack in adds about half. Natively, the same code wrote a deterministic 8 KB A4 PDF with Liberation Serif as a subset CID font with `ToUnicode` (`pdffonts`: emb/sub/uni all yes). `pdftotext` returned the text exactly, including the `fi`/`fl` ligatures. Recommendation: the export is a **second wasm module, loaded on first use**, so opening the page costs nothing. The fonts are separate fetches as well (Liberation Serif Regular is 393 KB as TTF).
4. **Two shapers.** krilla's `simple-text` brings rustybuzz. We shape with harfrust and give
   krilla glyphs, so `simple-text` stays off. If a transitive dependency drags it back in,
   `cargo tree -d` catches it.
5. **Pre-1.0 dependencies.** krilla and harfrust both move. Pin exact versions, and keep the
   whole krilla surface inside `pdf.rs` (one file, as `gdi.rs` is the only file creating a GDI
   object) so an upgrade touches one place.

---

## 8. The decisions

1. **krilla** over printpdf (§2). Recommended.
2. **Pagination as output, no page model** (§1), and the edits to `doc/not-doing.md` and
   `doc/text-layout.md` that come with it. Recommended.
3. **The first PDF printed the screen's faces (`look.rs`)**, and real paragraph styles came
   after, since resolving them changed the screen as well. Taken; the paragraph styles are now
   resolved for paper and screen alike.
4. **Bundled Liberation in native binaries, fetched on demand in the browser.** About 4 MB;
   the alternative is system fonts only, which makes the output depend on the machine.
5. **macOS previews with `PDFView`**, every other GUI client with our raster. Recommended.
6. **Windows prints raster pages.** Accepted as named, unless the user knows a Windows API
   worth the dependency.
7. **Paper is ISO 216 (DIN A).** The default page, when a document states none, is **A4
   portrait** with 2 cm margins. `--paper` names the A series only (`a3`, `a4`, `a5`, ...) and
   never infers a size from the locale. A document whose own page layout says US Letter is
   still printed on US Letter, because the document's own geometry always wins. What we never
   do is *choose* a US size.

---

## Sources

| | Where |
|---|---|
| The path this extends | `doc/text-layout.md`, Path C and "The decision" |
| The gates this amends | `doc/not-doing.md` §2 (Print to PDF; Pagination), `doc/text-core.md` (Styles; Fields) |
| Derived, never stored | `doc/view-modes.md` |
| One engine measures and draws | §2 |
| A display list executed by a backend | `ui_mac/src/ops.rs`, `doc/macos-shell.md` decision 3 |
| A filter's report as part of its output | `doc/xlsx-import.md` |
| `style:page-layout`, `style:page-layout-properties` | rng:12213, rng:12248 |
| `fo:page-width`, `fo:page-height` | rng:12255, rng:12260 |
| `style:master-page`, `office:master-styles`, `style:master-page-name` | rng:12141, rng:7939, rng:12803 |
| `fo:widows`, `fo:orphans`, `fo:keep-with-next` | rng:12512, rng:12517, rng:2110 |
| `style:header`, `style:footer` | rng:11962, rng:10844 |
