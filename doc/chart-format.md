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
turning ranges into shapes on screen. What stays out: the *chart's own* title and a legend, more
than one axis pair, stacked/percent variants, and every chart type beyond these three — each a
`chart:*` detail this build does not read or write, not a limitation of the mechanism. What an
*axis* carries is a different matter and is in scope — its own title, its tick labels and its
gridlines, three separate places in the file: see The axes, below.

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
     <chart:title><text:p>Party</text:p></chart:title>  <!-- rng:934, this axis' own title -->
     <chart:categories table:cell-range-address="Sheet1.B3:Sheet1.B9"/>   <!-- rng:454 -->
    </chart:axis>
    <chart:axis chart:dimension="y" …>
     <chart:title><text:p>Votes</text:p></chart:title>
    </chart:axis>
    <chart:series chart:class="chart:bar"              <!-- rng:857 -->
                  chart:values-cell-range-address="Sheet1.C3:Sheet1.C9"
                  chart:label-cell-address="Sheet1.C2:Sheet1.C2">
     <chart:data-point chart:style-name="ch7"/>         <!-- rng:553, one per bar (Bar, Pie) or
     <chart:data-point chart:style-name="ch8"/>              a run on a Line series, below -->
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

A series (line) or a data point (bar and pie — each bar or slice is its own point, since
neither has an axis to share a colour down between its own points) carries a `style:style
style:family="chart"` (rng:11059's `style:graphic-properties`, inside it) with:

- `draw:fill-color` (rng:10941) — a bar's fill, a line's own colour, a pie slice's fill.
- `svg:stroke-color` (rng:11102) — a line series repeats its fill colour here too, verified
  from the generated line chart; a bar chart's own style instead carries `draw:stroke="none"`.

LibreOffice's own defaults here are what prompted this feature's aesthetic requirement:
`#004586`, `#ff420e`, `#ffd320`, … — measured from the same generated charts, and not colours
this project chooses to reproduce. **This build's writer assigns [`grind_core::style::PALETTE`]
colours instead** (`doc/small-group.md`'s sibling rule applied to drawing rather than to
formulas: one named table, not a second one invented for charts), cycling a fixed, curated
order across a chart's own marks — one colour per bar or per slice (`Bar`, `Pie` — a bar
colours per point the same way a pie already did, rather than sharing one colour across a
whole series) or one per line (`Line`) — skipping the neutral entries (`black`, `white`,
`gray`, `silver`) that read as "no data" rather than as a colour.
[`grind_sheet::chart::effective_color`] is the single place this resolves, shared by the
writer and every shell's painter.

**A colour a user picks is a sticky override**, stored on [`crate::chart::Series`] (`color`
for a line series, `point_colors` for a bar or pie point) and written back verbatim on every
save — `App::set_chart_style` is the one entry point, reachable from the GTK shell (click a
mark, pick a swatch) and the CLI (`sheet chart-style --series-color`/`--point-color`). Reading
one back has to tell an override from an untouched default apart without a flag for it: the
reader compares a mark's own `draw:fill-color` to what `series_color` would compute for that
position and records an override only when they differ, so a chart nobody has touched keeps
re-cycling exactly as before (a series added or removed still reshuffles its neighbours'
colours) while a colour someone chose stays fixed regardless of what else in the chart
changes around it. The one gap this leaves: a user who happens to pick the colour the default
cycle would have produced anyway is indistinguishable from having picked nothing — harmless,
since the effective colour is identical either way.

### The axes — three things, three places in the file

An *axis* carries three things this build reads, writes and draws
([`grind_sheet::chart::Axis`]), and no two of them live in the same place:

| What | Where | Cited |
|---|---|---|
| Its title | `chart:axis`' own `chart:title`, a `text:p` inside it | rng:422-434 |
| Its tick labels | `chart:display-label`, on the **style** the axis names (`chart:style-name` → `style:style style:family="chart"` → `style:chart-properties`) | rng:10069, rng:446 |
| Its gridlines | a `chart:grid chart:class="major"` **element** inside `chart:axis` | rng:672-693 |

Child order inside `chart:axis` is the schema's, not this build's taste: title, then
categories, then any grid (rng:422-434). A minor grid is *read* as a grid — this build draws
one set of gridlines and writes the major one back — which is the same tolerance every other
distinction it does not model gets.

**`chart:display-label` is always written, on both axes, whichever way it goes.** Measured
(LibreOffice 26.2.5.2): a chart written with the attribute *absent* comes back out of
`soffice --convert-to` with `chart:display-label="false"` on both axes, and one written with
`"true"` comes back `"true"`. So an absent attribute is not "the default" to LibreOffice's
importer — it is *off*, and a chart written without it would draw its labels here and not
there. A chart LibreOffice itself writes states it explicitly too
(`sheet/tests/data/samples/Sales Dashboard.fods`, `ch4`/`ch5`), which is the same conclusion
reached from the other direction.

That measurement also fixes the **reader's** default, which is deliberately *not* the model's:
an axis a file says nothing about reads as [`grind_sheet::chart::Axis::bare`] — no tick labels
— because that is what the oracle draws for the same bytes. A **new** axis is
`Axis::default()` — tick labels on — which is a product decision about a chart somebody just
made, not a claim about a file.

Everything else an axis could carry is out, and out the same way the rest of the scope line is:
no secondary axis, no manual minimum or maximum (the scale is computed from the data —
[`grind_sheet::axis_ticks`], a step of 1, 2 or 5 times a power of ten, aiming for five
intervals), no logarithmic axis, no tick marks, no number format of its own, no label
rotation, no minor grid of its own.

**Where the scale lives.** `axis_ticks` is in `grind-sheet`, not in the shell that draws the
picture — the same call `doc/text-layout.md` made for line breaking, for the same reason: two
shells drawing the same chart against two different scales is two different charts. A shell
supplies only how wide a piece of text is.

## The shell

`grind-sheet-gtk` draws every chart on the active sheet over the cells it floats above
(`Grid::draw_charts`, called between `draw_cells` and `draw_filter_buttons` — over a cell's own
text, under the active-cell outline), reading `App::charts`/`App::chart_data` fresh each frame
the same way every other paint reads the document and throws it away (doc/plan.md rule 1).
`ui_sheet_gtk/src/chart.rs` is the drawing itself: a bar is `append_color` rectangles, a line and
a pie slice are `gsk::PathBuilder` paths stroked or filled — GTK's own vector drawing rather than
cairo, since nothing here needs more than straight edges (a pie slice is a many-sided polygon,
`PIE_SAMPLES_PER_TURN` fine enough that the seam does not show). Every mark's colour comes from
[`grind_sheet::chart::effective_color`], the same function the writer calls, so what is on
screen always matches what gets saved. No chart-level title and no legend — the same scope line
as the format itself, drawn rather than written. What an *axis* carries is drawn: its title
(centred under the plot for x, rotated beside it for y), its tick labels — the category names
under the x axis, the value scale beside the y one — and its gridlines, ruled under the marks
so a bar covers them rather than being cut by them.

**The plot is scaled to the top tick, not to the tallest bar** ([`grind_sheet::axis_ticks`]),
which is what lets the topmost gridline meet the top of the plot instead of floating below it.
Each axis takes its room out of the frame before anything is drawn: a title's fixed
`LABEL_SPACE`, the widest y tick label's own *measured* width, one line of x tick text. An x
label that would collide with the one before it is dropped rather than drawn over it
(`TICK_CLEARANCE`), so twenty categories in a narrow chart label the ones that fit. A pie has
no axes and keeps its whole frame whatever its axes carry.

Because a tick label's own width moves the plot, `draw` and `mark_at` both take the same
`Measure` — how wide and how tall a string is, in widget pixels, which is this shell's only
contribution to a chart's layout and exactly the shape `doc/text-layout.md` settled on for a
page of text. Measuring differently in the two would put the click somewhere the picture is not.

A toolbar button (**Chart**, labelled rather than icon-only — there is no chart icon in the
Adwaita icon theme, and the name this button first carried is not in it at all, so it drew as
the missing-image glyph) opens the dialog: chart kind, categories range, a repeatable list of
series (`RANGE[=LABEL]`, the same vocabulary `chart-add --series` already takes), and a group
per axis — title, tick labels, gridlines. **One dialog does both jobs**: inserting prefills
from the current selection when it spans more than one row and column (first column categories,
first row each series' own label, one series per remaining column) and calls `App::add_chart`;
editing prefills from the chart itself and calls `App::edit_chart`, one undo step, leaving the
position a drag put it at alone.

**Editing an existing chart is a double-click or a right-click.** A double-click anywhere on a
chart opens that dialog (the cell editor never opens underneath it, since a chart takes the
gesture first); a right-click opens a two-item menu — *Edit Chart…* and *Delete Chart* — built
the same way the column/row *Hide* menu is. Both hand the window a `Notice` rather than acting
in the widget: the dialog and the undo toast are the window's, and the grid's job is to say
which chart was asked about. Deleting is immediate with an Undo toast, exactly as deleting a
sheet is.

**Assigning a colour by hand is a click, not a dialog**:
`ui_sheet_gtk/src/chart.rs`'s `mark_at` shares the exact geometry `draw` paints from,
so a click on a bar, a slice or a line names the same mark the picture shows; a press that
never moved (`Grid`'s own click-vs-drag distinction, reused from the chart-drag gesture) opens
a palette popover — the same swatches a cell's fill-colour button offers
(`formatting::palette_grid`, factored out for this) — and picking one calls
`App::set_chart_style`, one undo step.

**Repositioning is a drag, not a dialog** — the feature this shell exists to get right where
LibreOffice's own frame-handle-and-recompute feel does not. Pressing on a chart's body starts a
move; pressing within its bottom-right handle (the same square the fill handle is) starts a
resize; both are pure presentation state (`Grid::chart_drag`/`chart_drag_rect`) painted in place
of the document's own geometry until the pointer is released, at which point the widget-space
rect becomes ODF lengths and one call to `App::reshape_chart` — one undo entry however long the
drag took, the same principle `Grid::commit_resize` already applies to a column or row. Nothing
is written mid-drag, which is what makes the drag itself smooth.

**Not built**: no visual feedback beyond the accent outline and handle already used for a
resize, no keyboard-driven repositioning — a mouse is what "dragged", "clicked" and
"right-clicked" all mean here — and no way to reach a chart's own dialog from the keyboard at
all, which is the a11y gap this feature leaves and the CLI's `chart-edit` covers for a script
but not for a person.

## What this build does not carry

No chart-level title, subtitle or legend (`chart-title`/`chart-subtitle`/`chart-legend`, each
`rng:optional` in `chart-chart`'s own content model) — read past and dropped, the same as any
other unmodelled optional element. **An axis' own title is different** — `chart:axis`'s own
`chart:title` (rng:422-434) is a distinct element from `chart:chart`'s, and is in scope: read,
written and drawn ([`grind_sheet::chart::Axis::label`]), along with the rest of what an axis
carries — see The axes, above. **Measured, not guessed**: a document this build writes with an
axis title is schema-valid and opens correctly in LibreOffice 26.2.5.2 (the same build
`ltwbw2026.*` was measured from), *and* survives a full `.fods` → `soffice --convert-to ods`
→ `soffice --convert-to fods` round trip through it with the title intact, on a bar chart and
on a pie alike — **correcting an earlier note here that said it did not**, which was measured
before the axis carried a style of its own and does not reproduce against that build.
No secondary axis, no stacked or
percent variants, no more than one categories range or values range per series. A chart's
embedded `table:table` (rng:463-483's own optional
`table-table` — the fallback data LibreOffice caches inside the chart document itself, in case
the ranges it points at ever go stale) is not read either: this build always re-resolves a
chart's ranges against the *live* sheet, the way a formula does, rather than trusting a cache
that can disagree with it.
