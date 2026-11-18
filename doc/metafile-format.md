<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# Windows metafiles, as read and measured

What `grind-print` reads of a WMF or EMF picture (`print/src/metafile/`). Record layouts are
Microsoft's Open Specifications, [MS-WMF] and [MS-EMF], as remembered rather than re-read: there
is no copy here, so each is marked `SPEC` only where LibreOffice 26.8.2.2's drawing of the
corpus's own metafiles agreed with it, and `MEASURED` where that drawing decided it.

The corpus is every WMF and EMF in the online samples' `.docx` and `.odt` files: 82, of which 81
draw something in LibreOffice. Each was put alone in a 12 cm frame and converted by both; at
50 dpi, 71 of them differ in less than a fifth of their inked pixels, and every one of the rest
was looked at (§6).

## 1. The two formats — `SPEC`

- **WMF**: an optional 22-byte placeable header (`9A C6 CD D7`, bounds as four `i16`, units to
  the inch as `u16`), then the metafile header (type 1 or 2, header size 9 words, version
  `0x0300`), then records: a size in 16-bit words, a function, parameters — many in reverse
  order (`META_LINETO` is *y*, *x*; `META_RECTANGLE` bottom, right, top, left). Objects take the
  lowest free slot of a table.
- **EMF**: records of a type and a size in bytes, the first `EMR_HEADER` with the picture's
  frame in hundredths of a millimetre at byte 24 and the reference device's size in pixels and
  millimetres at 72 and 80. Objects take the slot their record names; `0x80000000 | n` is
  stock object *n*.

Both are sniffed by their bytes (`grind_text::picture::mime`), so an ODT frame naming no type
draws one.

## 2. The picture's own size — `MEASURED`

A WMF is its placeable bounds at its units to the inch; an EMF is its frame. LibreOffice cut
both by `fo:clip` in those sizes (`doc/odt-format.md` §5c fact 64): a striped WMF 1000 units
wide at 1000 and at 500 to the inch, and the corpus EMF `6340…` cut down the middle of its
76.49 mm frame, each kept exactly the part the arithmetic says.

## 3. Coordinates — `SPEC`

EMF: a point goes through the world transform (`SetWorldTransform`, `ModifyWorldTransform`),
then the mapping mode — `MM_TEXT` one to one, `MM_LOMETRIC` … `MM_TWIPS` by the device's pixels
per millimetre with *y* upwards, `MM_ANISOTROPIC` by viewport over window, `MM_ISOTROPIC` by the
smaller of the two — and the frame's corner, in device pixels, comes off. WMF: by the window
alone onto the placeable bounds. `SaveDC`/`RestoreDC` keep all of it with the pen, brush, font,
colours, modes and clip.

## 4. Text — `MEASURED`

`ExtTextOut` places its string by `SetTextAlign`: baseline, top or bottom, and left, centre or
right of its advances when it gives them. LibreOffice puts the top and the bottom by the face's
**`hhea` ascent and descent**, and a positive `lfHeight` is the cell — that ascent and descent —
not the em: an `H` in Calibri set `TA_TOP` stood 0.745 em below the reference line and
`TA_BOTTOM` 0.25 em above it, Carlito's `hhea` 0.75 and 0.25, where its `usWinAscent` (0.952)
would have put it 0.2 em lower; in Arial, whose two agree, every case matched already. Calibri
is set in Carlito — the face a family resolves to here, through `grind-print`'s metric-compatible
twins, so a metafile's faces and characters join the families looked for. Each character goes at
its record's advance — except in a right-to-left script, whose letters must be shaped together:
there only the start is placed, and the corpus's Arabic lines then came out joined and ordered
as LibreOffice's. Half a surrogate pair, split between two records, draws nothing.

Not read: `ETO_GLYPH_INDEX` (glyph numbers in a face this does not have), a WMF's double-byte
charsets (its text is read as Latin-1), and a stock font's size — LibreOffice drew the corpus's
one stock-font line as large as the text around it, which a 12-unit default is not.

## 5. Lines, shapes, bitmaps, clips — `SPEC`

Pens (`PS_NULL` none, `PS_DASH` … `PS_DASHDOTDOT` dashed at Windows' cosmetic lengths, width
zero one device pixel), solid and null brushes, polygons, polylines, Béziers, rectangles,
ellipses, `MoveTo`/`LineTo`, paths (`BeginPath` … `FillPath`/`StrokePath`). Bitmaps are DIBs at 1,
4, 8, 16, 24 and 32 bits, or the JPEG or PNG one carries; a blit's source rectangle counts from
the top of the bitmap, `StretchDIBits`' from its bottom. `SRCCOPY` draws; an icon's pair —
`SRCAND` with its mask, then `SRCINVERT` or `SRCPAINT` into the same place — draws the second
where the mask is black, which is what the pair does on white paper; `PATCOPY`, `BLACKNESS` and
`WHITENESS` with no source fill. Clips: `IntersectClipRect`, `ExcludeClipRect` (the corpus's WMF
star is 400 of them), `SelectClipPath`, `ExtSelectClipRgn`'s rectangles, applied to shapes and
text and to bitmaps as a mask. The output keeps the drawing order: a bitmap between two shapes
is a layer between two SVGs.

Not read: EMF+ (the comments 12 corpus EMFs carry; only the plain EMF records beside them are
drawn, which for those 12 was enough to score with the rest),
other raster operations, pattern and hatched brushes (drawn solid), regions made by hand, arcs,
chords and pies, and the background `SetBkMode(OPAQUE)` paints behind text.

## 6. The ones that still differ

Of the ten scoring a fifth or more, each looked at side by side: six are the same picture a
pixel or two apart (`4837…`, `2959…`, `9e8c…`, `ff60…`, `40f0…`, and `6340…`'s four digits),
which a 50 dpi comparison of thin lines and small text counts in full; `2855…`, Arabic, matches
but for one line LibreOffice itself draws as separate letters; `910c…`, a test file
LibreOffice draws nothing of; `cb48…`, a test file whose glyph-index and stock-font lines are
§4's; and `9b9a…`, CJK text set in another fallback face than LibreOffice's.
