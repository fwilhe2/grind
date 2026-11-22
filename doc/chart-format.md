<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# Charts — clean-room notes and the scope line

Every citation below is `doc/OpenDocument-v1.4-schema.rng` by line. Measurements against a
real document say which file and which build of LibreOffice, per `CONTRIBUTING.md`'s clean-room
rule — LibreOffice's own source is never read for this, only its *output*, which is a
conformance oracle rather than a source.

## What is in scope

A chart type is in when it is useful: when real documents carry it, or people reach for it. There
is no fixed number. Each kind is one `chart:class` token and one way of turning ranges into
marks; the mechanism has no per-type special case. Built: bar, line, pie and area. A subtitle and
a footer, more than one axis pair and percent-stacked charts are not built yet. Each is a
`chart:*` detail this build does not read or write, not a limit of the mechanism. An axis'
title, tick labels and gridlines are built (The axes, below), and so are the chart's own title and
legend (The chart's own title and legend).

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

The inline shape is equally valid ODF regardless of physical form (rng:5541's choice says
nothing about packaging), and this build's writer used to use it for both, on the strength of
that. **Measured (LibreOffice 26.8.0.3, loop C's `charts` case): LibreOffice drops an inline
chart it finds in a package** — every chart in an `.ods` this build wrote was gone after one
`soffice --convert-to`, while the same charts in a `.fods` came through. So the writer follows
the form: **inline in the flat form** (R3 — one element, nothing to cross-reference) and **a
sub-document in the package form**, `Object N/content.xml` rooted at
`office:document-content`, pointed at by `draw:object xlink:href="./Object N"` with the
schema's `xlink:type="simple"` (rng:1621-1640) and declared in the manifest by a directory
entry carrying the chart's media type (`grind_core::odf::package::SubDocument`). No
`ObjectReplacements/` preview is written — LibreOffice regenerates one, and nothing here reads
it. Loop C now checks every chart in both forms, which is how this was found.

### Anchored to a cell

LibreOffice anchors a chart to a cell as often as to the sheet: the `draw:frame` sits inside the
`table:table-cell` instead of `table:shapes`, with `table:end-cell-address`, `table:end-x` and
`table:end-y` beside its size. **Measured** (LibreOffice 26.8.3.1, an Excel chart anchored 1cm
right of and 0.5cm below `E2`'s corner, converted to `.fods`): the frame went into `E2` with
`svg:x="0.3937in" svg:y="0.1768in"` — its offset from **that cell's** corner, not the sheet's.
This build reads such a frame as a chart placed at the cell's corner, summed from the track sizes
before it (a column the file sizes nowhere at LibreOffice's default 2.258cm, measured from its own
documents), plus that offset. Saved untouched, the frame stays in its cell, byte for byte; a save
that writes the sheet's charts again writes every one in `table:shapes` and takes the cell's
frame out (`source::Table::cell_charts`), so a chart is never written twice.

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
     <chart:data-point chart:style-name="ch7"/>         <!-- rng:553, one per slice (Pie), per
     <chart:data-point chart:style-name="ch8"/>              bar picked by hand (Bar), or a run -->
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
| `Area` | `chart:area` | LibreOffice 26.8.3.1's conversion of an Excel workbook with three fan charts (below) |

### Area, and a series of a kind of its own

Area came in with a Monte Carlo retirement
calculator exported from a spreadsheet service, whose three charts are **fan charts** — seven
stacked bands of an area chart, the percentiles of a simulation, with one or two lines (the
median, the need) drawn over them on the same axes. LibreOffice's conversion of it writes each as
`chart:chart chart:class="chart:area"` whose `chart:series` say `chart:class="chart:area"` for a
band and `chart:class="chart:line"` for a line (rng:892: a series has a class of its own), and
whose plot area's style says `chart:stacked="true"` (rng:9978). So the model carries three
things more, and nothing else:

- **`ChartKind::Area`**, read and drawn — each series a band at the categories' ticks.
- **`Series::kind`**, a series' own class where it differs from the chart's. Drawn in layers —
  bands, then bars, then lines, each over the last — and a pointer over a band names the band
  only where no line drawn over it is under the pointer.
- **`Chart::stacked`**. ODF has one flag for the whole plot area. **Measured** (LibreOffice
  26.8.3.1, a document of two area series of 10 and two line series of 15 and 25 under a stacked
  plot area, converted to a picture): the bands stand at 10 and 20 and the lines at 15 and 25 —
  only the series of the chart's own class stack. That is the rule drawn here for every kind but
  a pie: an area's bands stand on one another, a bar chart draws one column per category in
  segments, and a stacked line runs through the running totals.

Changing a chart's kind (Bar, Line, Pie, Area in every window) makes a plain chart of that kind:
`stacked` and every series' own kind go, since they belonged to the kind they were made for. An
edit that keeps the kind keeps both, a series' kind travelling with it as its colours do.

**A band's translucency is carried as the colour it shows.** Excel's fan charts fill every band
30% black so that gridlines show through; LibreOffice writes `draw:opacity="30%"` (rng:11039) on
the series' style. The model holds a colour and no opacity: the reader folds `draw:opacity` into
the fill over the chart's white (`chart::over_white`, `#000000` at 30% is `#b3b3b3`), which is
what a reader sees on a white chart, and a chart regenerated after an edit is written opaque. A
chart nobody edited is carried byte for byte, opacity and all (`sheet/tests/never_worse.rs`).

**A series named by text no cell holds** — every band of an imported fan chart is — carries the
text as `Series::name`, written as LibreOffice's `loext:label-string="&quot;Basis&quot;"`, the one
`loext:` item this build writes (`doc/plan.md`, R4). **Measured** (LibreOffice 26.8.3.1): it
writes the attribute for such a series and reads it back from no file, its own included, so
outside this build the legend says "Column B"; a name containing `"` has no spelling and
LibreOffice writes none.

**Measured, and a limit of the flat form in LibreOffice:** a chart on one sheet plotting a sheet
that comes *after* it in the document keeps its ranges when LibreOffice opens the package form,
and loses them when it opens the flat form — every series gone, or, in LibreOffice's own file,
turned into a static copy in the chart's `local-table`. An imported fan chart is exactly that
shape (the calculator sheet first, the calculation last), so such a workbook is for LibreOffice
best saved as `.ods`.

### Colour — verified, and where this build departs on purpose

A series and a data point each carry a `style:style style:family="chart"` (rng:11059's
`style:graphic-properties`, inside it) with:

- `draw:fill-color` (rng:10941) — a bar's fill, a line's own colour, a pie slice's fill.
- `svg:stroke-color` (rng:11102) — a line series repeats its fill colour here too, verified
  from the generated line chart; a bar chart's own style instead carries `draw:stroke="none"`.

**A data point's style is honoured only under a series that names one of its own.** Measured
(LibreOffice 26.8.0.3, by eye in its own window): a pie and a two-series bar chart this build
wrote — every bar and slice a `chart:data-point` with its own `chart:style-name`, the series
naming none — both drew in LibreOffice's *default* palette, every one of those styles ignored.
Adding `draw:fill="solid"` to them changed nothing. Adding a `chart:style-name` to the
`chart:series` element itself, and nothing else, made every data point's own colour appear. A
chart LibreOffice writes always names a series style
(`sheet/tests/data/samples/Sales Dashboard.fods`, `ch8`), which is why the earlier
measurement — made from LibreOffice's *output*, never from it reading ours — could not see it.
So **every series this build writes names a style**, whatever its kind.

LibreOffice's own defaults here are what prompted this feature's aesthetic requirement:
`#004586`, `#ff420e`, `#ffd320`, … — measured from the same generated charts, and not colours
this project chooses to reproduce. **This build's writer assigns [`grind_core::style::PALETTE`]
colours instead** (`doc/small-group.md`'s sibling rule applied to drawing rather than to
formulas: one named table, not a second one invented for charts), cycling a fixed, curated
order — skipping the neutral entries (`black`, `white`, `gray`, `silver`) that read as "no
data" rather than as a colour. **What a colour means is the one decision here, and it is: a
colour is a series.** A bar and a line colour per series, so two series side by side are two
colours and the legend can say which is which; a pie colours per slice, since a pie has one
series and its slices are what a reader tells apart. (Bars used to colour per *point*, the way
a pie does — which made Sales and Costs in the same group the same colour, a chart nobody
could read.) [`grind_sheet::chart::effective_color`] is the single place this resolves, shared
by the writer and every shell's painter.

**A colour a user picks is a sticky override**, stored on [`crate::chart::Series`] (`color` for
a whole bar or line series, `point_colors` for one bar or one pie slice) and written back
verbatim on every save — `App::set_chart_style` is the one entry point, reachable from the GTK
shell (click a mark, pick a swatch) and the CLI (`sheet chart-style --series-color`/
`--point-color`). A bar series' data points are written only where one carries an override,
with `chart:repeated` over the runs between them; a pie's are always written, one per slice.
Reading one back has to tell an override from an untouched default apart without a flag for
it: the reader compares a series' own `draw:fill-color` to what `series_color` would compute
for that series (a slice's to what it would compute for that slice, a bar's to its series'
colour) and records an override only when they differ, so a chart nobody has touched keeps
re-cycling exactly as before (a series added or removed still reshuffles its neighbours'
colours) while a colour someone chose stays fixed regardless of what else in the chart
changes around it. The one gap this leaves: a user who happens to pick the colour the default
cycle would have produced anyway is indistinguishable from having picked nothing — harmless,
since the effective colour is identical either way.

### Direction — a pie says which way it goes, and this build always says so

A pie has an *angle* axis, the chart's y axis, and which way round it runs is that axis'
`chart:reverse-direction` (rng:10064), a `style:chart-properties` attribute on the style the
axis names — the same place `chart:display-label` lives, below. Measured (LibreOffice
26.8.0.3, by eye, the same four-slice pie written three ways):

| The y axis' style says | LibreOffice draws |
|---|---|
| nothing | **counter-clockwise** from twelve o'clock — the first slice just *left* of twelve |
| `chart:reverse-direction="true"` | **clockwise** from twelve o'clock — the first slice just right of twelve |

So a pie whose file says nothing runs right to left there, and that is what a pie this build
wrote used to do: drawn clockwise in this window, counter-clockwise in LibreOffice, from the
same bytes. The fix has two halves, like `chart:display-label`'s:

- **The writer always states it** on a pie's y axis, whichever way it goes, so no reader has a
  default to disagree about. A **new** pie is clockwise ([`grind_sheet::Chart::clockwise`]) —
  a product decision, the direction a clock and a reader both go.
- **The reader takes the oracle's default**: a pie whose file says nothing reads as
  counter-clockwise, because that is how LibreOffice draws the same bytes, and a pie drawn one
  way here and the other way there is the bug this section exists for.

A bar or line chart's own axes may carry the attribute too (LibreOffice writes
`chart:reverse-direction="false"` on both, `Sales Dashboard.fods`'s `ch4`/`ch5`); on the
category axis it would run the categories right to left. This build neither reads nor writes
it there — a named gap, and one no chart this build makes can reach, since it only writes the
attribute for a pie. The slices' own angles are computed once, in
[`grind_sheet::chart::pie_slices`], for the reason [`grind_sheet::axis_ticks`] is: two shells
sweeping the same pie two ways is two different charts.

### The chart's own title and legend

Two children of `chart:chart` itself, ahead of the plot area in the schema's own order — title,
subtitle, footer, legend, plot area (rng:462-485):

| What | Where | Cited |
|---|---|---|
| The title | `chart:title` with one `text:p` — the same definition an axis' title uses, one level up | rng:466, rng:934-956 |
| The legend | `chart:legend chart:legend-position="start|end|top|bottom"` | rng:475, rng:694-758 |

LibreOffice writes both with a position and a style of their own
(`sheet/tests/data/samples/Sales Dashboard.fods`'s title carries `svg:x`/`svg:y` and
`chart:style-name="ch2"`; `spreadsheet.fods`'s legend adds `style:legend-expansion="high"`).
**This build writes neither** — `svg:x`/`svg:y` are optional (rng:1722-1733), and so is the
legend's expansion (rng:735's choice includes `rng:empty`). Measured (LibreOffice 26.8.0.3, loop
C's `charts` case): a title and a legend at the end or at the bottom, written with nothing but
their text and their position, come back from `soffice --convert-to` exactly as written, and
LibreOffice lays both out itself. Reading back, a legend in one of the schema's corners
(`top-start` and so on, rng:718-725) reads as the edge it is on, and one that names no position
at all reads as `end`, where LibreOffice draws it. A title is plain text, and read with ODF's
white-space rule — LibreOffice pretty-prints a chart's `text:p`, so without it a title read back
carried the newline and indentation in front of it.

**What a new chart gets** is a product decision, [`grind_sheet::ChartSpec::default_legend`]: a
legend at the end when there is more than one thing to tell apart — two or more series, or a
pie's slices — and none for a single bar or line series, whose one colour is what the chart is
*of*. A chart made from a table ([`grind_sheet::chart::guess`]) takes that single series' name
as its title. Both are the dataviz rule stated the other way round: a legend is always there
when colour is carrying identity, and never there to restate a title.

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
**Where every mark goes is the core's** (`grind_sheet::chart_paint`): a chart is a list of
marks — fills, polygons, polylines and text — that every graphical shell draws with its own
toolkit, so the same chart is the same picture in each. A bar is a polygon at most `BAR_MAX`
(24 units) wide, `BAR_GAP` (2) from its neighbour, rounded at its data end and square at the
baseline, which is the `dataviz` skill's mark spec; a line is a 2-unit stroke; a pie slice is a
many-sided polygon swept by `pie_slices`. Every mark's colour comes from
[`grind_sheet::chart::effective_color`], the same function the writer calls, so what is on
screen always matches what gets saved. The chart's own title is a bold band across the top;
its legend is a swatch and a name per series (per slice for a pie) at its edge, the name in ink
and never in the series' colour. Both take their room before the axes do, and a legend that
would crowd a small chart out is left off rather than squeezing the plot. What an *axis* carries is drawn: its title
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

A shell's only contribution to a chart's layout is a `Metrics` — how wide a string is — the
shape `doc/text-layout.md` settled on for a page of text. `chart_paint::picture` keeps where
each bar, line, slice and legend entry went, so a click (`Picture::target_at`) and a tooltip
(`Picture::describe`) ask the geometry the marks were placed by.

**The dialog is built around a live preview** (`ui_sheet_gtk/src/chart_dialog.rs`), reached
from the cells' context menu (*Insert Chart…*) or the palette. **One dialog does both jobs**,
insert and edit, and its top third is the chart — drawn by the same `chart::draw` the grid uses,
from `App::preview_chart`, which is `add_chart`/`edit_chart` without the write, so what the
dialog shows and what lands on the sheet cannot differ. Below it, three buttons pick the type
(a glyph and a word each, since Adwaita has no chart icons), and three groups of ordinary
Adwaita rows say the rest:

- **Data** asks what the chart is of in the table's own terms — the block, whether the series
  run in columns or rows, and whether the first row and first column are labels — which are
  `ChartShape`'s three answers, turned into ranges by `App::suggest_chart` (the core's
  `chart::guess`, the same reading `chart-add --from` makes). Opening *Insert Chart* on any cell
  of a table reads the whole table; a table with months across its top charts them along the x
  axis without being told. The ranges themselves — `RANGE=LABEL`, `chart-add --series`'s own
  vocabulary — are folded away under *Ranges*, for anybody who wants a series the table's shape
  does not give. A range that does not parse turns its row red and the preview says why.
- **Labels**: the chart's title, its legend (None, Right, Bottom, Top, Left — ODF's
  start/end, in the words of a left-to-right window), and for a pie whether it runs clockwise.
  A title and a legend follow the table until the person picks their own.
- **Axes**: one expander per axis — title, labels, gridlines — hidden for a pie.

Insert and Apply sit in the header bar, Cancel opposite, Enter in any text row inserts, and the
write is one undo step. **A new chart goes beside the block it was made from** — a column's gap
to the right of its last column, level with its first row (`Grid::anchor_beside`, from the
sheet's own track sizes with the zoom taken back out) — and is scrolled into view
(`Grid::reveal_chart`). It used to land at a fixed spot near the top-left corner, on top of the
figures it had just been made from. **Editing** opens on the chart as it is: its ranges shown as
typed, the table they span shown as the range, and nothing re-read until the person changes the
range or how it is read, so opening and applying a chart leaves it exactly as it was.

**Editing an existing chart is a double-click or a right-click.** A double-click anywhere on a
chart opens that dialog (the cell editor never opens underneath it, since a chart takes the
gesture first); a right-click opens a two-item menu — *Edit Chart…* and *Delete Chart* — built
the same way the column/row *Hide* menu is. Both hand the window a `Notice` rather than acting
in the widget: the dialog and the undo toast are the window's, and the grid's job is to say
which chart was asked about. Deleting is immediate with an Undo toast, exactly as deleting a
sheet is.

**Assigning a colour by hand is a click, not a dialog**:
`Picture::target_at` is the geometry the marks were drawn by, so a click on a bar, a slice, a line or a legend entry names the same mark the picture shows; a
press that never moved (`Grid`'s own click-vs-drag distinction, reused from the chart-drag
gesture) opens a palette popover — the same swatches a cell's fill-colour button offers
(`formatting::palette_grid`, factored out for this) — and picking one calls
`App::set_chart_style`, one undo step. **A colour is a series**, so a click on a bar colours its
whole series; the popover's *Only this bar* check is the deliberate exception. A pie's slice is
always its own.

**Hovering a mark names it**: a tooltip reads `Sales · Feb: 20` over a bar or a line and
`Feb: 20 (33%)` over a slice (`chart::describe`, the same hit-test again), which is how a figure
comes off a chart without a label on every bar. A chart under the pointer wears a faint outline
and a move cursor; a *selected* one wears the accent and all eight handles for as long as it is
selected (`doc/chart-handling.md`, which every shell now follows).

**Repositioning is a drag, not a dialog** — the feature this shell exists to get right where
LibreOffice's own frame-handle-and-recompute feel does not. Pressing on a chart's body starts a
move; pressing on any of the selected chart's eight handles starts a resize from that edge or
corner, the opposite one held (`grind_sheet::chart_frame`, shared with every other shell); both are pure presentation state (`Grid::chart_drag`/`chart_drag_rect`) painted in place
of the document's own geometry until the pointer is released, at which point the widget-space
rect becomes ODF lengths and one call to `App::reshape_chart` — one undo entry however long the
drag took, the same principle `Grid::commit_resize` already applies to a column or row. Nothing
is written mid-drag, which is what makes the drag itself smooth.

**From the keyboard**, *Edit a Chart…* in the palette opens the dialog on the sheet's chart,
or offers the sheet's charts by title when there are several — which closes the gap this
section used to name, that a chart's dialog could not be reached without a pointer at all.
**Not built**: keyboard-driven repositioning and resizing (a mouse is what "dragged" means
here, and `chart-reshape` is the keyboard's way), and a chart the keyboard can select and delete
without the context menu.

## What this build does not carry

No subtitle or footer (`chart-subtitle`/`chart-footer`, each `rng:optional` in `chart-chart`'s
own content model) — read past and dropped, the same as any other unmodelled optional element.
The chart's own title and legend are carried (above), and so is an axis' own title —
`chart:axis`'s own `chart:title` (rng:422-434), read, written and drawn
([`grind_sheet::chart::Axis::label`]) along with the rest of what an axis carries. A title's or a
legend's own position and style are not: LibreOffice's placement of either is re-derived, the
way a chart's colours are. **Measured, not guessed**: a document this build writes with an
axis title is schema-valid and opens correctly in LibreOffice 26.2.5.2 (the same build
`ltwbw2026.*` was measured from), *and* survives a full `.fods` → `soffice --convert-to ods`
→ `soffice --convert-to fods` round trip through it with the title intact, on a bar chart and
on a pie alike — **correcting an earlier note here that said it did not**, which was measured
before the axis carried a style of its own and does not reproduce against that build.
No secondary axis, no percent
variants, no more than one categories range
or values range per series. A chart's
embedded `table:table` (rng:463-483's own optional
`table-table` — the fallback data LibreOffice caches inside the chart document itself, in case
the ranges it points at ever go stale) is not read either: this build always re-resolves a
chart's ranges against the *live* sheet, the way a formula does, rather than trusting a cache
that can disagree with it.
