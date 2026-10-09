// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **Taking hold of a chart** — `grind_sheet::chart_frame`'s one behaviour, in this page
//! (`doc/chart-handling.md`).
//!
//! A press on a chart selects it, and a selected chart wears an accent outline and eight square
//! handles (`.chart.selected`, `.chart-handle`) for as long as it is selected. Its body moves it,
//! a handle resizes it with the opposite edge held, Shift on a corner keeps its proportions; the
//! arrows nudge it, Delete and Backspace delete it, Return changes it, Escape lets go. A right
//! click selects it and opens a menu saying all of that with its keys. Nothing is written while
//! the pointer is down: the page draws the frame the drag has reached, and the release is one
//! `App::reshape_chart`, one Ctrl+Z.
//!
//! The DOM does the hit-testing — a handle is its own element, its reach widened by a
//! `::before` of [`chart_frame::HANDLE_SLOP`] — so the only arithmetic here is `chart_frame`'s.
//! Frames are in the chart layer's own pixels, which CSS `zoom` scales: a pointer's movement in
//! the window is divided by the zoom before it moves anything.

use grind_sheet::chart_frame::{self, Frame, Grip};
use grind_sheet::style::{length_mm, mm_length};
use wasm_bindgen::prelude::*;
use web_sys::{Element, HtmlElement, KeyboardEvent, MouseEvent};

use super::layout::{self, PX_PER_MM, Tracks};
use super::{Ui, attribute};

/// A chart being dragged: which, by what, where it started and where the pointer was, whether
/// it was already selected, and the frame the drag has reached so far.
#[derive(Clone, Copy, Debug)]
pub(super) struct Grab {
    index: usize,
    grip: Grip,
    start: Frame,
    from: (f64, f64),
    now: Frame,
}

/// The column of row headers' width, in the layer's pixels — `.grid`'s corner `<col>` (3.5rem).
const HEADER_W: f64 = 3.5 * 16.0;

impl Ui {
    /// Every chart's frame on this sheet in the layer's pixels (`None` for one whose lengths
    /// this build cannot read, which is a chart it does not draw), and where the sheet's own
    /// corner is in the same pixels — what turns a frame back into `svg:x`/`svg:y`.
    pub(super) fn chart_frames(
        &self,
        widths: &Tracks,
        heights: &Tracks,
    ) -> (Vec<Option<Frame>>, (f64, f64)) {
        let scroll = self.scroll.get();
        let origin = (
            HEADER_W - widths.span(0, scroll.col),
            layout::CELL.cell_h - heights.span(0, scroll.row),
        );
        let charts = self.app.charts(self.sheet.get()).unwrap_or_default();
        let px = |length: &str| length_mm(length).map(|mm| mm * PX_PER_MM);
        let frames = charts
            .iter()
            .map(|chart| {
                Some(Frame::new(
                    origin.0 + px(&chart.x)?,
                    origin.1 + px(&chart.y)?,
                    px(&chart.width)?,
                    px(&chart.height)?,
                ))
            })
            .collect();
        (frames, origin)
    }

    /// Where chart `index` is drawn this frame: the drag's own frame while it is being dragged.
    pub(super) fn chart_drawn_at(&self, index: usize, frame: Frame) -> Frame {
        match self.chart_grab.get() {
            Some(grab) if grab.index == index => grab.now,
            _ => frame,
        }
    }

    /// The selected chart, if it still exists — an index left over from a chart deleted or
    /// undone away is let go of rather than pointing at the next one.
    pub(super) fn selected_chart_now(&self) -> Option<usize> {
        let index = self.chart_selected.get()?;
        let count = self
            .app
            .charts(self.sheet.get())
            .map_or(0, |charts| charts.len());
        if index >= count {
            self.chart_selected.set(None);
            return None;
        }
        Some(index)
    }

    /// The chart a chart verb means: the selected one, else the sheet's last.
    pub(super) fn chart_target(&self) -> Option<usize> {
        self.selected_chart_now().or_else(|| {
            self.app
                .charts(self.sheet.get())
                .ok()
                .and_then(|charts| charts.len().checked_sub(1))
        })
    }

    /// Select a chart, or (with `None`) give the keyboard back to the cells, and redraw the
    /// layer if that changed anything.
    pub(super) fn select_chart(&self, index: Option<usize>) {
        if self.chart_selected.replace(index) != index {
            if index.is_some() {
                self.set_message(
                    "Chart selected — drag to move, a handle to resize, Delete deletes it"
                        .to_owned(),
                );
            }
            self.redraw_charts();
        }
    }

    fn redraw_charts(&self) {
        if let Err(error) = self.render_charts(&self.widths(), &self.heights()) {
            web_sys::console::error_1(&error);
        }
    }

    /// The eight handles of a selected chart drawn at `frame`, appended to the layer beside the
    /// chart rather than inside it, since `.chart` clips what overflows it and half of every
    /// handle is outside.
    pub(super) fn draw_handles(&self, index: usize, frame: &Frame) -> Result<(), JsValue> {
        for (number, (grip, square)) in chart_frame::handles(frame, chart_frame::HANDLE)
            .into_iter()
            .enumerate()
        {
            let handle = self.dom.document.create_element("div")?;
            handle.set_class_name("chart-handle");
            handle.set_attribute("data-chart", &index.to_string())?;
            handle.set_attribute("data-grip", &number.to_string())?;
            handle.set_attribute(
                "style",
                &format!(
                    "left:{:.1}px;top:{:.1}px;width:{:.1}px;height:{:.1}px;cursor:{}",
                    square.x,
                    square.y,
                    square.w,
                    square.h,
                    grip.cursor()
                ),
            )?;
            self.dom.charts.append_child(&handle)?;
        }
        Ok(())
    }

    /// What a press on `target` has of a chart: a handle, a body, or nothing.
    fn chart_under(target: &Element) -> Option<(usize, Grip)> {
        if let Ok(Some(handle)) = target.closest(".chart-handle") {
            let grip = *Grip::HANDLES.get(attribute(&handle, "data-grip")? as usize)?;
            return Some((attribute(&handle, "data-chart")? as usize, grip));
        }
        let chart = target.closest(".chart").ok()??;
        Some((attribute(&chart, "data-chart")? as usize, Grip::Body))
    }

    /// A press on the grid. On a chart it selects the chart and — with the main button — takes
    /// hold of it, and the press is the chart's (`true`). Anywhere else it lets go of a selected
    /// chart and is the grid's as before.
    pub(super) fn chart_press(
        &self,
        event: &MouseEvent,
        target: &Element,
    ) -> Result<bool, JsValue> {
        self.close_chart_menu();
        let Some((index, grip)) = Self::chart_under(target) else {
            self.select_chart(None);
            return Ok(false);
        };
        if self.editing.get() {
            self.commit(None)?;
        }
        self.dragging.set(false);
        // No text selection, no focus moving into the SVG.
        event.prevent_default();
        self.select_chart(Some(index));
        let (frames, _) = self.chart_frames(&self.widths(), &self.heights());
        if event.button() == 0
            && let Some(Some(start)) = frames.get(index)
        {
            self.chart_grab.set(Some(Grab {
                index,
                grip,
                start: *start,
                from: (f64::from(event.client_x()), f64::from(event.client_y())),
                now: *start,
            }));
        }
        self.dom.surface.focus()?;
        Ok(true)
    }

    /// The pointer moving with a chart held: the frame follows it, nothing is written. `true`
    /// when there was a chart to move, so the grid's own drag does not run as well.
    pub(super) fn chart_drag(&self, event: &MouseEvent) -> bool {
        let Some(mut grab) = self.chart_grab.get() else {
            return false;
        };
        let zoom = self.zoom.get();
        let dx = (f64::from(event.client_x()) - grab.from.0) / zoom;
        let dy = (f64::from(event.client_y()) - grab.from.1) / zoom;
        let keep = event.shift_key() && grab.grip.is_corner();
        grab.now =
            chart_frame::dragged(&grab.start, grab.grip, dx, dy, chart_frame::MIN_SIZE, keep);
        self.chart_grab.set(Some(grab));
        self.redraw_charts();
        true
    }

    /// The button coming up on a held chart: a press that barely moved only selected it; any
    /// other is one `App::reshape_chart`.
    pub(super) fn chart_release(&self) {
        let Some(grab) = self.chart_grab.take() else {
            return;
        };
        let (dx, dy) = (grab.now.x - grab.start.x, grab.now.y - grab.start.y);
        let (dw, dh) = (grab.now.w - grab.start.w, grab.now.h - grab.start.h);
        let slop = chart_frame::CLICK_SLOP;
        if chart_frame::is_click(dx, dy, slop) && chart_frame::is_click(dw, dh, slop) {
            self.redraw_charts();
            return;
        }
        self.write_chart_frame(grab.index, grab.now);
    }

    /// A frame in the layer's pixels written back as ODF lengths, kept on the sheet — one
    /// `App::reshape_chart`, one undo step.
    fn write_chart_frame(&self, index: usize, frame: Frame) {
        let (_, origin) = self.chart_frames(&self.widths(), &self.heights());
        let mm = |px: f64| px / PX_PER_MM;
        let on_sheet = chart_frame::kept_on_sheet(Frame::new(
            mm(frame.x - origin.0),
            mm(frame.y - origin.1),
            mm(frame.w),
            mm(frame.h),
        ));
        let result = self.app.reshape_chart(
            self.sheet.get(),
            index,
            &mm_length(on_sheet.x),
            &mm_length(on_sheet.y),
            &mm_length(on_sheet.w.max(0.1)),
            &mm_length(on_sheet.h.max(0.1)),
        );
        match result {
            Ok(()) => self.set_message("Chart moved — Ctrl+Z puts it back".to_owned()),
            Err(error) => self.set_message(error.to_string()),
        }
    }

    /// A key while a chart is selected and no cell is being edited (`chart_frame::Key`): Delete
    /// and Backspace delete it, Escape lets go, the arrows nudge it (Shift for a large step),
    /// Return changes it. Any other key lets go of it and means what it always means — `false`.
    pub(super) fn chart_key(&self, event: &KeyboardEvent) -> bool {
        if self.editing.get() {
            return false;
        }
        let Some(index) = self.selected_chart_now() else {
            return false;
        };
        let shift = event.shift_key();
        let key = match event.key().as_str() {
            "Delete" | "Backspace" => chart_frame::Key::Delete,
            "Escape" => chart_frame::Key::Deselect,
            "ArrowLeft" => chart_frame::nudge(-1, 0, shift),
            "ArrowRight" => chart_frame::nudge(1, 0, shift),
            "ArrowUp" => chart_frame::nudge(0, -1, shift),
            "ArrowDown" => chart_frame::nudge(0, 1, shift),
            "Enter" => {
                event.prevent_default();
                self.restyle_chart();
                return true;
            }
            "Shift" | "Control" | "Alt" | "Meta" => return false,
            _ => {
                self.select_chart(None);
                return false;
            }
        };
        event.prevent_default();
        match key {
            chart_frame::Key::Delete => self.delete_chart(),
            chart_frame::Key::Deselect => {
                self.select_chart(None);
                self.set_message(String::new());
            }
            chart_frame::Key::Nudge(dx, dy) => {
                let (frames, _) = self.chart_frames(&self.widths(), &self.heights());
                if let Some(Some(frame)) = frames.get(index) {
                    let moved = Frame::new(frame.x + dx, frame.y + dy, frame.w, frame.h);
                    self.write_chart_frame(index, moved);
                }
            }
        }
        true
    }

    /// A right click: on a chart it selects it and opens the chart's own menu at the pointer,
    /// and the browser's menu does not open. `true` when it was a chart's.
    pub(super) fn chart_context_menu(&self, event: &MouseEvent) -> Result<bool, JsValue> {
        let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
            return Ok(false);
        };
        let Some((index, _)) = Self::chart_under(&target) else {
            self.close_chart_menu();
            return Ok(false);
        };
        event.prevent_default();
        self.chart_grab.set(None);
        self.select_chart(Some(index));
        let menu: &HtmlElement = &self.dom.chart_menu;
        menu.style()
            .set_property("left", &format!("{}px", event.client_x()))?;
        menu.style()
            .set_property("top", &format!("{}px", event.client_y()))?;
        menu.set_hidden(false);
        if let Some(first) = menu.query_selector("button")? {
            first.unchecked_into::<HtmlElement>().focus()?;
        }
        Ok(true)
    }

    pub(super) fn close_chart_menu(&self) {
        self.dom.chart_menu.set_hidden(true);
    }
}
