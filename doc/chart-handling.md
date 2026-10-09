<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Chart handling — taking hold of a chart on the grid

**Status: normative, and built in every client** (2026-10-09). `sheet/src/chart_frame.rs` is the
behaviour; each shell's half is named below. What a chart *is* — its ranges, kind, title, legend,
colours, and how it is written — is `doc/chart-format.md`. This document is only about the hand
on it: selecting a chart, moving it, resizing it, deleting it.

## Why it is its own document

LibreOffice is where this went wrong, and the person this suite is for says so plainly: a chart
there is hard to tell you have hold of, its handles are small and appear late, and the gestures
change with what mode the chart is in. The cure is not a cleverer gesture. It is **one set of
rules, decided once, in the core**, so that every window answers "which handle is under the
pointer", "where does this drag put the chart" and "what does Delete do now" with the same
function rather than six answers that drift.

## The rules

1. **A click selects a chart.** It does not move it, edit it or open anything. A press and
   release that moved less than `CLICK_SLOP` (4 CSS px at 1×) is a click.
2. **A selected chart is unmistakable, for as long as it is selected**: an outline in the accent
   colour and **eight square handles** (`HANDLE`, 9 CSS px — the page colour inside an accent
   border), centred on its corners and edges so half of each sits outside the chart, where it
   reads as a handle rather than as part of the picture. They do not wait for the pointer to come
   near a corner. An unselected chart under the pointer may show a faint outline, saying it can be
   picked up; it shows no handles and has none to grab.
3. **The body moves it; a handle resizes it with the opposite edge held still.** A handle answers
   `HANDLE_SLOP` (4 px) beyond its drawn square — easier to grab than to see, never harder — and
   the selected chart's handles are asked before any chart's body, since they reach outside it.
   Among bodies, the chart drawn on top (the last in `table:shapes`' order) answers first.
4. **Shift on a corner keeps the proportions**, following whichever axis the pointer moved
   further along. A resize never makes a chart smaller than `MIN_SIZE` (32 px); the edge being
   dragged stops, the far one does not move to make room.
5. **Nothing is written while the pointer is down.** The frame the drag has reached is drawn; the
   release is **one** `App::reshape_chart` — one undo step, whether it moved or resized.
6. **A chart is never dragged or nudged past A1** (`kept_on_sheet`): `svg:x` and `svg:y` are
   offsets from the sheet's corner and a negative one is a chart nobody can see.
7. **The keys on a selected chart** are the same everywhere: Delete and Backspace delete it, the
   arrows nudge it (`NUDGE` 2 px, Shift `NUDGE_LARGE` 20 px), Return changes it (the shell's own
   way of saying kind, title and legend), Escape lets go. **Any other key lets go of the chart and
   means what it always means** — typing does not get swallowed by a selection you forgot about.
8. **Deleting says how to get it back** (`deleted_sentence`): a key that deletes is only safe when
   the way back is obvious.
9. **A right click on a chart selects it first**, then opens the chart's own menu, which shows the
   keys beside the items they share (Return, Delete). What the menu acts on is the chart that is
   outlined.
10. **The chart verbs mean the selected chart, else the sheet's last.** A palette entry, a menu
    item or a command typed with nothing selected still does something sensible, and with a chart
    selected it does it to that one.
11. **Selection is presentation**, like the cell selection: never written, let go of when the
    sheet changes or the chart is deleted or undone away (each shell asks "is the index still on
    this sheet and still in range" before trusting it).

Every number above is in **CSS pixels at 1×**; a shell scales it into its own unit — the DPI on
Windows, points on the Mac (taken as they are), the zoom where a size is *on the sheet* (the
minimum, a nudge) but not where it is chrome (a handle).

## The shells

| Client | Where | Notes |
|---|---|---|
| GNOME | `ui_sheet_gtk/src/grid.rs` (`chart_hit`, `chart_key`, `draw_charts`) | The first. A second click on a selected chart's body opens the colour popover for the mark under it. Not yet seen on a display from this machine (no Xvfb) |
| Browser | `ui_web/src/sheet/grab.rs` | The DOM hit-tests: a handle is its own element (`.chart-handle`), its reach widened by a `::before` of the slop. `#chart-menu` is the right-click menu; a double click changes the chart. The `#charts` layer is CSS-zoomed, so a pointer's movement is divided by the zoom. `smoke.js` drives select, resize, Ctrl+Z, Escape, Delete and the menu in jsdom |
| Windows | `ui_win32/src/sheet/chart.rs` (portable: `frames`, `hit`, `Grab`, `lengths`, `nudged`; GDI: `paint_held`), `win.rs` (`chart_press`, `chart_key`, `context_on_chart`) | Capture on press; `WM_SETCURSOR` maps a grip to `IDC_SIZEALL`/`IDC_SIZENS`/`IDC_SIZEWE`/`IDC_SIZENESW`/`IDC_SIZENWSE`; the Data menu's three chart verbs and a chart's own context menu. The handles are drawn over the cell selection and clipped to the cells. Seen in a Wine `--render-to` frame; the drag itself is unit-tested, not driven |
| Mac | `ui_mac/src/sheet/chart.rs` (portable: `Held`, `hit`, `Grab`, `lengths`, `key`, `held_ops`), `grid_view.rs` (`grab_chart`, `chart_key`, `change_chart`) | Keys arrive as selectors (`deleteBackward:`, `cancelOperation:`, `moveLeftAndModifySelection:`, `insertNewline:`); cursors are cursor rects — the open hand on a body, `frameResizeCursorFromPosition:inDirections:` on each handle (macOS 15, the floor). Return or a double click pops the chart's menu under its corner; ⌫ is shown beside Delete Chart. Type-checked for both Apple targets, not yet run |
| Terminal | `ui_tui/src/sheet/chartpane.rs` | No pointer and no chart on the grid, so the gesture is a **list**: `:charts` shows every chart as a row, the selected one reversed and drawn in characters below. `d`/`x`/Delete deletes it, `HJKL` nudge it a cell, `+`/`-` scale it 1.2×, `m` moves it to the cursor, Enter opens `:chart ` for it, Esc lets go. It stays selected when the pane closes, so `:chart <words>`, `:chart here` and `:chart!` mean it |
| CLI | `grind sheet chart-list`, `chart-reshape`, `chart-remove` | By index, which is what every shell's selection is underneath |

## Named gaps

- **No multiple selection.** One chart at a time, in every client; aligning two charts is two
  drags.
- **No snapping** to cell edges or to other charts. A drag lands where the pointer lets go.
- **The terminal places by a rough geometry** (a column 25 mm, a row 5 mm), the same one `:chart`
  and `:chart here` always used, so its "at C1" is approximate where the document's own widths
  differ.
- **Keyboard selection of a chart** exists only in the terminal's pane. In the pointer shells a
  chart is selected by clicking it (or right-clicking it); Tab does not cycle charts.
