// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The colour grid both panes pick from — one popover, moved under whichever button opened it.
//!
//! The colours are [`grind_core::style::PALETTE`], the core's own table, so a swatch here and
//! `sheet style --color navy` on the CLI are the same attribute. A page could offer a native
//! `<input type="color">` instead and hand back an arbitrary hex; that is a *different*
//! product decision (`doc/sheet-shell.md`: a palette is a default a shell offers, never a
//! limit) and this shell has not made it — what the grid cannot say, the document can still
//! hold, and every colour a file already had is drawn as it is.

use std::cell::RefCell;

use grind_core::style::PALETTE;
use wasm_bindgen::prelude::*;
use web_sys::{Document, Element, HtmlElement, MouseEvent};

use crate::{element, listen};

/// Which colour a pick is for. The shell matches on it, the same way it matches a command id.
pub type Target = String;

pub struct Swatches {
    document: Document,
    root: HtmlElement,
    /// What the currently open grid will set — `None` while it is closed.
    target: RefCell<Option<Target>>,
}

impl Swatches {
    pub fn find(document: &Document) -> Result<Self, JsValue> {
        Ok(Swatches {
            document: document.clone(),
            root: element(document, "swatches")?,
            target: RefCell::new(None),
        })
    }

    pub fn is_open(&self) -> bool {
        !self.root.hidden()
    }

    pub fn target(&self) -> Option<Target> {
        self.target.borrow().clone()
    }

    pub fn close(&self) {
        self.root.set_hidden(true);
        *self.target.borrow_mut() = None;
    }

    /// Open the grid under `anchor`, setting `target` when something is picked.
    ///
    /// Positioned in fixed coordinates from the button's own rectangle rather than nested
    /// inside it: a popover inside the tool row would be clipped by it.
    pub fn open(&self, anchor: &Element, target: Target) -> Result<(), JsValue> {
        self.build()?;
        *self.target.borrow_mut() = Some(target);
        let at = anchor.get_bounding_client_rect();
        let style = self.root.style();
        style.set_property("left", &format!("{}px", at.left()))?;
        style.set_property("top", &format!("{}px", at.bottom() + 4.0))?;
        self.root.set_hidden(false);
        Ok(())
    }

    /// The grid itself, rebuilt on each opening — seventeen buttons is cheaper to make than
    /// to keep in step with a theme that may have changed underneath it.
    fn build(&self) -> Result<(), JsValue> {
        self.root.set_text_content(None);
        for (name, hex) in PALETTE {
            let button = self.document.create_element("button")?;
            button.set_attribute("type", "button")?;
            button.set_attribute("data-color", hex)?;
            button.set_attribute("title", name)?;
            button.set_attribute("style", &format!("background:{hex}"))?;
            let label = self.document.create_element("span")?;
            label.set_class_name("sr");
            label.set_text_content(Some(name));
            button.append_child(&label)?;
            self.root.append_child(&button)?;
        }
        // Clearing is a colour too — the one the document does not name.
        let none = self.document.create_element("button")?;
        none.set_attribute("type", "button")?;
        none.set_class_name("none");
        none.set_attribute("data-color", "")?;
        none.set_text_content(Some("Default"));
        self.root.append_child(&none)?;
        Ok(())
    }
}

/// Wire the grid's clicks to `chosen`, which is handed `(target, Option<hex>)` — `None` for
/// the *Default* row, which is what clearing the property spells.
pub fn wire(
    swatches: &std::rc::Rc<Swatches>,
    chosen: impl Fn(Target, Option<String>) + 'static,
) -> Result<(), JsValue> {
    let owner = swatches.clone();
    listen(&swatches.root, "mousedown", move |event: MouseEvent| {
        let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
            return;
        };
        let Ok(Some(button)) = target.closest("button") else {
            return;
        };
        let Some(what) = owner.target() else { return };
        // The button must not take the focus off the document being coloured.
        event.prevent_default();
        let hex = button.get_attribute("data-color").unwrap_or_default();
        owner.close();
        chosen(what, (!hex.is_empty()).then_some(hex));
    })
}
