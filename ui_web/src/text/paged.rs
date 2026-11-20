// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The document as its printed pages (`doc/page-model.md` PM7): a canvas per page, drawn by the
//! print module's `Pages`, which keeps the document typeset and answers every question about
//! where its text is in the strip's CSS pixels. This module holds the canvases and asks; it knows
//! no font.

use std::collections::HashSet;

use grind_text::Caret;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d, Document, HtmlCanvasElement, HtmlElement};

/// The print module's `Pages`, and the canvases its pages are drawn on.
pub struct Paged {
    pages: JsValue,
    /// The document's change count as of the last setting; `None` before the first.
    seen: Option<u64>,
    /// Pages that changed since they were last drawn — drawn when they come into view.
    stale: HashSet<usize>,
    canvases: Vec<HtmlCanvasElement>,
    /// Device pixels per point the canvases were drawn at.
    drawn: f64,
}

/// Call `name` on a JS object with `args`.
fn call(this: &JsValue, name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    let function: js_sys::Function = js_sys::Reflect::get(this, &name.into())?.dyn_into()?;
    let array = js_sys::Array::new();
    for arg in args {
        array.push(arg);
    }
    function.apply(this, &array)
}

/// A typed array of numbers the print module answered with, or `None` for `undefined`.
fn numbers(value: &JsValue) -> Option<Vec<f64>> {
    if value.is_undefined() || value.is_null() {
        return None;
    }
    Some(js_sys::Float64Array::new(value).to_vec())
}

fn caret(pair: &[f64]) -> Option<Caret> {
    Some(Caret {
        block: *pair.first()? as usize,
        offset: *pair.get(1)? as usize,
    })
}

fn pair(at: Caret) -> JsValue {
    js_sys::Uint32Array::from(&[at.block as u32, at.offset as u32][..]).into()
}

/// A rectangle in the strip: x, y, width, height.
pub type Rect = [f64; 4];

impl Paged {
    pub fn new(pages: JsValue) -> Self {
        Paged {
            pages,
            seen: None,
            stale: HashSet::new(),
            canvases: Vec::new(),
            drawn: 0.0,
        }
    }

    /// Set the pages again from the document's bytes when it changed since they were.
    pub fn sync(&mut self, changes: u64, bytes: impl FnOnce() -> Option<Vec<u8>>) {
        if self.seen == Some(changes) {
            return;
        }
        let Some(bytes) = bytes() else { return };
        let bytes: JsValue = js_sys::Uint8Array::from(bytes.as_slice()).into();
        match call(&self.pages, "set", &[bytes]).as_ref().map(numbers) {
            Ok(Some(damaged)) => {
                self.stale.extend(damaged.into_iter().map(|at| at as usize));
                self.seen = Some(changes);
            }
            Ok(None) => self.seen = Some(changes),
            Err(error) => web_sys::console::error_1(error),
        }
    }

    /// Place the pages at `scale` CSS pixels per point in a view `width` wide.
    pub fn place(&self, scale: f64, width: f64) {
        let _ = call(&self.pages, "place", &[scale.into(), width.into()]);
    }

    fn ask(&self, name: &str, args: &[JsValue]) -> Option<Vec<f64>> {
        numbers(&call(&self.pages, name, args).ok()?)
    }

    pub fn count(&self) -> usize {
        call(&self.pages, "count", &[])
            .ok()
            .and_then(|count| count.as_f64())
            .map_or(0, |count| count as usize)
    }

    /// The strip's width and height.
    pub fn size(&self) -> (f64, f64) {
        match self.ask("size", &[]).as_deref() {
            Some(&[width, height]) => (width, height),
            _ => (0.0, 0.0),
        }
    }

    pub fn sheet(&self, page: usize) -> Option<Rect> {
        self.ask("sheet", &[(page as u32).into()])?.try_into().ok()
    }

    pub fn visible(&self, top: f64, bottom: f64) -> std::ops::Range<usize> {
        match self.ask("visible", &[top.into(), bottom.into()]).as_deref() {
            Some(&[from, to]) => from as usize..to as usize,
            _ => 0..0,
        }
    }

    /// The caret's stroke: x, top, height in the strip, and its x on its page in points.
    pub fn spot(&self, at: Caret) -> Option<[f64; 4]> {
        let args = [(at.block as u32).into(), (at.offset as u32).into()];
        self.ask("spot", &args)?.try_into().ok()
    }

    pub fn hit(&self, x: f64, y: f64) -> Option<Caret> {
        caret(&self.ask("hit", &[x.into(), y.into()])?)
    }

    pub fn step(&self, at: Caret, delta: isize, goal: f64) -> Option<Caret> {
        let args = [
            (at.block as u32).into(),
            (at.offset as u32).into(),
            (delta as i32).into(),
            (goal as f32).into(),
        ];
        caret(&self.ask("step", &args)?)
    }

    pub fn bounds(&self, at: Caret) -> Option<(Caret, Caret)> {
        let args = [(at.block as u32).into(), (at.offset as u32).into()];
        let ends = self.ask("bounds", &args)?;
        Some((caret(&ends)?, caret(ends.get(2..)?)?))
    }

    /// The text between two carets on the pages between `top` and `bottom`, a rectangle a line.
    pub fn bands(&self, from: Caret, to: Caret, top: f64, bottom: f64) -> Vec<Rect> {
        self.ask("bands", &[pair(from), pair(to), top.into(), bottom.into()])
            .unwrap_or_default()
            .as_chunks::<4>()
            .0
            .to_vec()
    }

    pub fn blocks(&self, top: f64, bottom: f64) -> Option<std::ops::Range<usize>> {
        match self.ask("blocks", &[top.into(), bottom.into()])?.as_slice() {
            &[from, to] => Some(from as usize..to as usize),
            _ => None,
        }
    }

    /// A canvas for every page in `into`, placed and sized, and the ones between `top` and
    /// `bottom` drawn if they changed or the scale did. `ratio` is the screen's device pixels per
    /// CSS pixel, so a page is as sharp as the screen.
    pub fn draw(
        &mut self,
        document: &Document,
        into: &HtmlElement,
        (top, bottom): (f64, f64),
        scale: f64,
        ratio: f64,
    ) -> Result<(), JsValue> {
        let count = self.count();
        while self.canvases.len() > count {
            if let Some(canvas) = self.canvases.pop() {
                canvas.remove();
            }
        }
        while self.canvases.len() < count {
            let canvas: HtmlCanvasElement = document.create_element("canvas")?.dyn_into()?;
            canvas.set_class_name("sheet");
            into.append_child(&canvas)?;
            self.stale.insert(self.canvases.len());
            self.canvases.push(canvas);
        }
        let device = scale * ratio;
        if (device - self.drawn).abs() > 1e-9 {
            self.stale.extend(0..count);
            self.drawn = device;
        }
        for (page, canvas) in self.canvases.iter().enumerate() {
            let Some([x, y, width, height]) = self.sheet(page) else {
                continue;
            };
            let style = canvas.style();
            style.set_property("left", &format!("{x}px"))?;
            style.set_property("top", &format!("{y}px"))?;
            style.set_property("width", &format!("{width}px"))?;
            style.set_property("height", &format!("{height}px"))?;
        }
        for page in self.visible(top, bottom) {
            if !self.stale.remove(&page) {
                continue;
            }
            let Some(canvas) = self.canvases.get(page) else {
                continue;
            };
            // A page with no 2d context to draw on (a headless host) keeps its place and its
            // marks; only its pixels are missing.
            let Some(context) = canvas.get_context("2d")? else {
                continue;
            };
            let context: CanvasRenderingContext2d = context.dyn_into()?;
            let sheet = call(
                &self.pages,
                "render",
                &[(page as u32).into(), device.into()],
            )?;
            if sheet.is_undefined() {
                continue;
            }
            let number = |name: &str| {
                js_sys::Reflect::get(&sheet, &name.into())
                    .ok()
                    .and_then(|value| value.as_f64())
                    .unwrap_or(0.0) as u32
            };
            let (width, height) = (number("width"), number("height"));
            let rgba = js_sys::Reflect::get(&sheet, &"rgba".into())
                .map_or_default(|bytes| js_sys::Uint8Array::new(&bytes).to_vec());
            canvas.set_width(width);
            canvas.set_height(height);
            // A page is opaque — white paper under everything — so its premultiplied pixels are
            // the straight ones a canvas expects.
            let image = web_sys::ImageData::new_with_u8_clamped_array_and_sh(
                wasm_bindgen::Clamped(&rgba),
                width,
                height,
            )?;
            context.put_image_data(&image, 0.0, 0.0)?;
        }
        Ok(())
    }

    /// A different document arrived: set it whole, and draw every page.
    pub fn forget(&mut self) {
        self.seen = None;
        self.stale.extend(0..self.canvases.len());
    }
}
