// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The autofilter's dropdown: the list of values behind a header cell's button (§9.4).
//!
//! The core decides what a filter *means* (`core/src/filter.rs`); this decides only how it
//! is picked. The one rule that matters here is that the values offered are the strings
//! [`grind_sheet::Filter::hides`] compares against — both come from `App`'s single `render`,
//! reached through [`grind_sheet::App::get_viewport`], so the list cannot offer a value that would then
//! match nothing.
//!
//! Read from the **document**, never from what is drawn: a value whose rows the filter
//! currently hides has to stay in the list, or unchecking it would be a one-way door.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;

use libadwaita::gtk;
use libadwaita::prelude::*;

use grind_sheet::{Filter, Viewport};

use crate::geom::Rect;

/// How many distinct values the list will show.
///
/// ponytail: a column with more than this many distinct values gets a truncated list and no
/// way to reach the rest. The upgrade is a search entry over the values, which is also what
/// makes a list this long usable at all — worth building the first time a real document
/// wants it, not before.
const MAX_VALUES: usize = 500;

/// What the empty cell is called in the list. A blank row would read as a rendering bug, and
/// the value behind it really is the empty string (`core/src/filter.rs`).
const EMPTY_LABEL: &str = "(empty)";

/// The distinct values a field's column holds, in the order the list shows them.
///
/// A `BTreeSet` so the order is the model's own — [`Filter::keep`] stores its values the
/// same way, and a list that reordered itself between openings is its own small bug. Pure,
/// and the reason the popover below has nothing to decide.
pub fn field_values(cells: &Viewport, filter: &Filter, field: u32) -> Vec<String> {
    let col = filter.column(field);
    (filter.first_data_row()..=filter.end.row)
        .filter_map(|row| cells.text(row, col))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(MAX_VALUES)
        .collect()
}

/// What the dropdown decided, handed back to whoever opened it.
pub enum Chosen {
    /// Keep exactly these values in this field.
    Keep(BTreeSet<String>),
    /// Drop this field's condition — every value shows again.
    Clear,
}

/// The dropdown itself: one popover, reused for every field.
pub struct FilterMenu {
    popover: gtk::Popover,
    list: gtk::ListBox,
    all: gtk::CheckButton,
    apply: gtk::Button,
    /// The value behind each row's checkbox, in the order they were added.
    checks: RefCell<Vec<(String, gtk::CheckButton)>>,
    field: Cell<u32>,
    #[allow(clippy::type_complexity)]
    on_apply: RefCell<Option<Box<dyn Fn(u32, Chosen)>>>,
    /// Set while the code — rather than the user — is ticking boxes, so the two handlers
    /// below do not chase each other round the "all" checkbox.
    settling: Cell<bool>,
}

impl FilterMenu {
    pub fn new(parent: &impl IsA<gtk::Widget>) -> std::rc::Rc<Self> {
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("boxed-list");

        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .propagate_natural_height(true)
            .propagate_natural_width(true)
            .max_content_height(280)
            .min_content_width(180)
            .build();

        let all = gtk::CheckButton::with_label("Select all");
        let clear = gtk::Button::with_label("Clear");
        clear.set_tooltip_text(Some("Show every row again"));
        let apply = gtk::Button::with_label("Apply");
        apply.add_css_class("suggested-action");

        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        buttons.set_homogeneous(true);
        buttons.append(&clear);
        buttons.append(&apply);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
        content.append(&all);
        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        content.append(&scroller);
        content.append(&buttons);

        let popover = gtk::Popover::builder()
            .child(&content)
            .has_arrow(true)
            .position(gtk::PositionType::Bottom)
            .build();
        popover.set_parent(parent);

        let menu = std::rc::Rc::new(Self {
            popover,
            list,
            all,
            apply,
            checks: RefCell::new(Vec::new()),
            field: Cell::new(0),
            on_apply: RefCell::new(None),
            settling: Cell::new(false),
        });

        menu.all.connect_toggled({
            let menu = std::rc::Rc::downgrade(&menu);
            move |all| {
                let Some(menu) = menu.upgrade() else { return };
                if menu.settling.get() {
                    return;
                }
                menu.settling.set(true);
                for (_, check) in menu.checks.borrow().iter() {
                    check.set_active(all.is_active());
                }
                menu.settling.set(false);
                menu.refresh_apply();
            }
        });

        clear.connect_clicked({
            let menu = std::rc::Rc::downgrade(&menu);
            move |_| {
                let Some(menu) = menu.upgrade() else { return };
                menu.popover.popdown();
                menu.emit(Chosen::Clear);
            }
        });

        menu.apply.connect_clicked({
            let menu = std::rc::Rc::downgrade(&menu);
            move |_| {
                let Some(menu) = menu.upgrade() else { return };
                menu.popover.popdown();
                menu.emit(Chosen::Keep(menu.ticked()));
            }
        });

        menu
    }

    /// What to do when the dropdown decides. Set once; the field number comes back with it,
    /// so one callback serves every column.
    pub fn connect_apply(&self, f: impl Fn(u32, Chosen) + 'static) {
        *self.on_apply.borrow_mut() = Some(Box::new(f));
    }

    /// A popover has to be unparented before its parent goes away.
    pub fn dispose(&self) {
        self.popover.unparent();
    }

    pub fn is_visible(&self) -> bool {
        self.popover.is_visible()
    }

    /// Show the list for one field under its button. `kept` is the field's current condition
    /// — `None` when it has none, which ticks everything, because a field nobody has filtered
    /// keeps every value it has.
    pub fn open(
        self: &std::rc::Rc<Self>,
        at: Rect,
        field: u32,
        values: &[String],
        kept: Option<&BTreeSet<String>>,
    ) {
        self.field.set(field);
        while let Some(row) = self.list.first_child() {
            self.list.remove(&row);
        }

        let mut checks = Vec::with_capacity(values.len());
        for value in values {
            let label = match value.is_empty() {
                true => EMPTY_LABEL,
                false => value.as_str(),
            };
            let check = gtk::CheckButton::with_label(label);
            check.set_active(kept.is_none_or(|k| k.contains(value)));
            if value.is_empty() {
                // Italic, so "(empty)" cannot be confused with a cell that literally says it.
                if let Some(label) = check.last_child().and_downcast::<gtk::Label>() {
                    label.set_markup(&format!("<i>{EMPTY_LABEL}</i>"));
                }
            }
            check.set_margin_start(6);
            check.set_margin_end(6);
            check.connect_toggled({
                let menu = std::rc::Rc::downgrade(self);
                move |_| {
                    let Some(menu) = menu.upgrade() else { return };
                    if menu.settling.get() {
                        return;
                    }
                    menu.sync_all();
                    menu.refresh_apply();
                }
            });
            self.list.append(&check);
            checks.push((value.clone(), check));
        }
        *self.checks.borrow_mut() = checks;
        self.sync_all();
        self.refresh_apply();

        self.popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
            at.x as i32,
            at.y as i32,
            at.w as i32,
            at.h as i32,
        )));
        self.popover.popup();
    }

    fn ticked(&self) -> BTreeSet<String> {
        self.checks
            .borrow()
            .iter()
            .filter(|(_, check)| check.is_active())
            .map(|(value, _)| value.clone())
            .collect()
    }

    /// The "Select all" box reflects the rows rather than driving them: all, none, or the
    /// inconsistent state in between.
    fn sync_all(&self) {
        let checks = self.checks.borrow();
        let ticked = checks.iter().filter(|(_, c)| c.is_active()).count();
        self.settling.set(true);
        self.all
            .set_inconsistent(ticked > 0 && ticked < checks.len());
        self.all.set_active(ticked == checks.len() && ticked > 0);
        self.settling.set(false);
    }

    /// Applying nothing would hide every row — a document that has apparently emptied itself.
    /// The button says no instead.
    fn refresh_apply(&self) {
        let any = self.checks.borrow().iter().any(|(_, c)| c.is_active());
        self.apply.set_sensitive(any);
        self.apply.set_tooltip_text(match any {
            true => None,
            false => Some("Keep at least one value, or Clear to show every row"),
        });
    }

    fn emit(&self, chosen: Chosen) {
        if let Some(f) = self.on_apply.borrow().as_ref() {
            f(self.field.get(), chosen);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_sheet::{App, CellValue, Pos};

    /// The list is the column's distinct display text — deduplicated, in the model's own
    /// order, and *including* the values the filter is currently hiding.
    #[test]
    fn the_list_offers_every_value_the_column_holds() {
        let app = App::new();
        for (row, product) in ["Product", "Desk", "Chair", "Desk", ""].iter().enumerate() {
            app.set_cell(
                0,
                Pos::new(row as u32, 1),
                CellValue::Text((*product).to_owned()),
            )
            .unwrap();
        }
        let mut filter = Filter::new("f", Pos::new(0, 1), Pos::new(4, 1));
        // Chair is filtered out, so its rows are hidden — and it still has to be offered.
        filter.keep.insert(0, ["Desk".to_owned()].into());

        let cells = app.get_viewport(0, 0..5, 0..3).unwrap();
        assert_eq!(
            field_values(&cells, &filter, 0),
            vec!["".to_owned(), "Chair".to_owned(), "Desk".to_owned()],
            "the heading is not one of its own values, and an empty cell is one"
        );
    }

    /// The strings offered are the ones the core matches on, which is the whole contract
    /// between this module and `core/src/filter.rs`.
    #[test]
    fn every_offered_value_matches_something() {
        let app = App::new();
        app.set_cell(0, Pos::new(0, 0), "Price").unwrap();
        // A number, so the display text is a rendering rather than the stored value.
        app.set_cell(0, Pos::new(1, 0), CellValue::Number(1.5))
            .unwrap();
        app.set_cell(0, Pos::new(2, 0), CellValue::Number(2.0))
            .unwrap();
        let filter = Filter::new("f", Pos::new(0, 0), Pos::new(2, 0));
        let cells = app.get_viewport(0, 0..3, 0..1).unwrap();

        for value in field_values(&cells, &filter, 0) {
            let mut one = filter.clone();
            one.keep.insert(0, [value.clone()].into());
            let hidden = one.hidden_rows(
                // The sheet behind the app, read the same way the grid reads it.
                &grind_sheet::read_bytes(
                    "x.fods",
                    &app.save_bytes(grind_sheet::Form::Flat).unwrap(),
                )
                .unwrap()
                .sheets[0],
                0,
            );
            assert!(
                hidden.len() < 2,
                "keeping {value:?} hid every data row, so the list offered a value \
                 the filter cannot match"
            );
        }
    }
}
