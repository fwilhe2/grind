<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Charts — clean-room notes and the scope line

Every citation below is `doc/OpenDocument-v1.4-schema.rng` by line. Measurements against a
real document say which file and which build of LibreOffice, per `CONTRIBUTING.md`'s clean-room
rule — LibreOffice's own source is never read for this, only its *output*, which is a
conformance oracle rather than a source.

## What is in scope, and why three

`doc/not-doing.md` originally drew the line at one chart type — "one that round-trips proves
the mechanism, the second is taste." That line moved by an explicit decision: bar, line and pie
are the three shapes a spreadsheet's data most commonly wants, and building one showed that the
second and third add no new mechanism, only a second `chart:class` token and a different way of
turning ranges into shapes on screen. What stays out: chart *titles*, a *legend*, more than one
axis pair, stacked/percent variants, and every chart type beyond these three — each a `chart:*`
detail this build does not read or write, not a limitation of the mechanism.

## The two places a chart's own document can live

A chart is a **complete second ODF document** (`office:mimetype=
"application/vnd.oasis.opendocument.chart"`), embedded inside a `draw:object` (rng:5539) that
sits inside a `draw:frame` (rng:5088) that sits inside a `table:shapes` (rng:15678) that is a
sibling of a sheet's rows, inside `table:table`. `draw:object`'s content is a **choice**
(rng:5541-5545): a reference (`common-draw-data-attlist`, `xlink:href`) or an **inline**
`office:document` (rng:7799) — the schema does not care which, in either physical form of the
outer document, which is the fact this build relies on:

- **The flat form (`.fods`) embeds it inline** — `<draw:object><office:document
  office:mimetype="…chart">…</office:document></draw:object>`, no `xlink:href` needed at all.
  Measured from `ltwbw2026.fods` (LibreOffice 26.2.5.2).
- **The package form (`.ods`) LibreOffice writes references a separate part** —
  `<draw:object xlink:href="./Object 1" …>`, `Object 1/content.xml` inside the zip (rooted at
  `office:document-content` rather than `office:document`, exactly as the outer document's own
  `content.xml` is), declared in `META-INF/manifest.xml`, plus a `draw:image` sibling pointing at
  `ObjectReplacements/Object 1` — a static preview bitmap nothing here reads, since this build
  draws a chart from its own model rather than from a cached picture. Measured from
  `ltwbw2026.ods`, same source document as the `.fods` above.

Because the inline shape is equally valid ODF regardless of physical form (rng:5541's choice
says nothing about packaging), **this build's writer always uses it — R3 applied to embedding,
the same call `doc/odt-format.md` made for an image's frame.** A chart this build wrote loads
into LibreOffice unchanged; a chart LibreOffice wrote and this build re-emits on a regenerating
save comes back the simpler shape, exactly the trade the image writer already makes.

## What a chart's own document holds

```xml
<office:document office:mimetype="application/vnd.oasis.opendocument.chart" …>
 <office:body><office:chart>                          <!-- rng:7717 / rng:7687 -->
  <chart:chart chart:class="chart:bar"                <!-- rng:463, class verified below -->
               svg:width="16cm" svg:height="9cm">
   <chart:plot-area chart:style-name="ch3" …>          <!-- rng:776 -->
    <chart:axis chart:dimension="x" …>                 <!-- rng:423 -->
     <chart:categories table:cell-range-address="Sheet1.B3:Sheet1.B9"/>   <!-- rng:454 -->
    </chart:axis>
    <chart:axis chart:dimension="y" …/>
    <chart:series chart:class="chart:bar"              <!-- rng:857 -->
                  chart:values-cell-range-address="Sheet1.C3:Sheet1.C9"
                  chart:label-cell-address="Sheet1.C2:Sheet1.C2">
     <chart:data-point chart:style-name="ch7"/>         <!-- rng:553, one per point or … -->
     <chart:data-point chart:repeated="7"/>              <!-- … or one run, when every point
                                                                shares its series' own colour -->
    </chart:series>
    <chart:wall/><chart:floor/>                          <!-- rng:958, rng:645 -->
   </chart:plot-area>
  </chart:chart>
 </office:chart></office:body>
</office:document>
```

`chart:categories`'s and `chart:series`'s own range attributes are `cellRangeAddressList`
(rng:395), a space-separated list of `cellRangeAddress` (rng:382) — `["]sheet-name["].`
`[$]COL[$]ROW[:[.\.\.].[$]COL[$]ROW]`, the same grammar `sheet/src/a1.rs` already parses for a
formula reference, minus the `[…]` a user's own typed address needs and this attribute never
has. Read and written as one range each (this build's charts have one categories range and one
values range per series) via `a1::parse_bracketed`, a small refactor pulling the tail of
`a1::parse` (lexing, not the case-folding a *user's* address needs) into its own function so
there is one address parser rather than two.

### `chart:class` — verified, not guessed

The schema leaves `chart:class` a free `namespacedToken` (rng:487-489) — nothing in the RNG
enumerates the tokens LibreOffice actually uses, so each one below is measured rather than
assumed, from a real `soffice --headless` (26.2.5.2, matching `ltwbw2026.*`) building a chart of
each kind through its own UNO API (`ScTabViewShell`'s chart insertion, `LineDiagram`/
`PieDiagram`/`BarDiagram` respectively) and saving flat:

| This build's [`ChartKind`] | `chart:class` | Measured from |
|---|---|---|
| `Bar` | `chart:bar` | `ltwbw2026.fods`, a real document |
| `Line` | `chart:line` | A `soffice`-built line chart, same LibreOffice build |
| `Pie` | `chart:circle` | A `soffice`-built pie chart, same LibreOffice build — "circle" is
  ODF's own name for a pie chart, and it is the one surprising token of the three |

### Colour — verified, and where this build departs on purpose

A series (bar, line) or a data point (pie — each slice is its own point, since a pie has no
axis to share a colour down) carries a `style:style style:family="chart"` (rng:11059's
`style:graphic-properties`, inside it) with:

- `draw:fill-color` (rng:10941) — a bar's fill, a line's own colour, a pie slice's fill.
- `svg:stroke-color` (rng:11102) — a line series repeats its fill colour here too, verified
  from the generated line chart; a bar chart's own style instead carries `draw:stroke="none"`.

LibreOffice's own defaults here are what prompted this feature's aesthetic requirement:
`#004586`, `#ff420e`, `#ffd320`, … — measured from the same generated charts, and not colours
this project chooses to reproduce. **This build's writer assigns [`grind_core::style::PALETTE`]
colours instead** (`doc/small-group.md`'s sibling rule applied to drawing rather than to
formulas: one named table, not a second one invented for charts), cycling a fixed, curated
order across a chart's series (bar, line — one colour per series) or its data points (pie — one
per slice), skipping the neutral entries (`black`, `white`, `gray`, `silver`) that read as
"no data" rather than as a colour. A chart this build only *read* keeps whatever colours the
file already had — this only applies to a chart's own XML being regenerated by this writer, the
same "verbatim until it changes" rule every other style in this codebase follows.

## What this build does not carry

No chart title, subtitle or legend (`chart-title`/`chart-subtitle`/`chart-legend`, each
`rng:optional` in `chart-chart`'s own content model) — read past and dropped, the same as any
other unmodelled optional element. No secondary axis, no stacked or percent variants, no more
than one categories range or values range per series, no `chart:data-point` beyond the colour a
regenerated chart assigns. A chart's embedded `table:table` (rng:463-483's own optional
`table-table` — the fallback data LibreOffice caches inside the chart document itself, in case
the ranges it points at ever go stale) is not read either: this build always re-resolves a
chart's ranges against the *live* sheet, the way a formula does, rather than trusting a cache
that can disagree with it.
