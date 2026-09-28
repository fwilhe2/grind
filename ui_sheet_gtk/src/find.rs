// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The find bar — Ctrl+F, and Ctrl+H with its replace row open: `grind sheet find` and
//! `grind sheet replace` in this window.
//!
//! A `gtk::SearchBar` under the formula bar, the shape `ui_text_gtk/src/find.rs` gave the word
//! processor: it slides in, it searches as it is typed, Enter and Shift+Enter walk the hits
//! across every sheet (wrapping at either end), and Escape puts the keyboard back in the grid
//! with the hit it stopped on still selected.
//!
//! **Nothing here decides what a match is.** Every question goes to `App::find` and every
//! write to `App::replace` (`grind_sheet::find`): a cell is searched through its input text, so
//! `sum` finds `=SUM(B2:B9)`, and a replaced cell goes back in through the typing rule. The
//! *Replace* button is the same call narrowed to one cell — `Search::range` with both corners on
//! the hit — so a formula this window's replace would break is refused by the core, not by a
//! second copy of that rule here.
//!
//! Where this sits in `doc/sheet-shell.md`'s "Four surfaces": it is none of them. It is
//! *transient* — the verb that opens it is a row in `main.rs`'s `actions()`, and the bar goes
//! away again when it is done — so it adds nothing to the chrome that stays on screen.
//!
//! The hits are asked for again at every step rather than kept, because the document may have
//! been edited between two presses of Enter. Which hit is next is `grind_sheet::find::step`,
//! shared with the browser's F3.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use libadwaita::gtk;
use libadwaita::prelude::*;

use grind_sheet::find::{Search, Towards, step};
use grind_sheet::{App, Pos, RecalcMode};
use gtk::glib;

use crate::grid::{Grid, Notice};
use crate::keymap::Selection;

/// A cell, in the order the core reports hits: sheet, then row, then column.
pub type Place = (usize, Pos);

/// What a replace tells the window, which owns the toasts.
pub type Said = Box<dyn Fn(&str)>;

/// The bar and the widgets in it anything outside needs.
pub struct Find {
    pub bar: gtk::SearchBar,
    entry: gtk::SearchEntry,
    count: gtk::Label,
    replacing: gtk::ToggleButton,
    with: gtk::Entry,
    match_case: gtk::ToggleButton,
    whole_cell: gtk::ToggleButton,
    app: Arc<App>,
    grid: Grid,
    /// How a finished replace is announced — `Ui::undoable_toast`, set once the window exists.
    said: RefCell<Option<Said>>,
}

impl Find {
    pub fn new(app: &Arc<App>, grid: &Grid) -> Rc<Self> {
        let entry = gtk::SearchEntry::builder()
            .placeholder_text("Find in every sheet")
            .hexpand(true)
            .build();
        let count = gtk::Label::new(None);
        count.add_css_class("dim-label");
        count.add_css_class("numeric");
        let previous = gtk::Button::builder()
            .icon_name("go-up-symbolic")
            .tooltip_text("Previous (Shift+Enter)")
            .build();
        let next = gtk::Button::builder()
            .icon_name("go-down-symbolic")
            .tooltip_text("Next (Enter)")
            .build();
        let arrows = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        arrows.add_css_class("linked");
        arrows.append(&previous);
        arrows.append(&next);

        // The two ways a search narrows, as the CLI's `--match-case` and `--whole-cell`.
        let match_case = gtk::ToggleButton::builder()
            .label("Aa")
            .tooltip_text("Match Case")
            .build();
        let whole_cell = gtk::ToggleButton::builder()
            .label("Whole Cell")
            .tooltip_text("The whole cell, not a piece of it")
            .build();
        let options = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        options.add_css_class("linked");
        options.append(&match_case);
        options.append(&whole_cell);

        let replacing = gtk::ToggleButton::builder()
            .icon_name("edit-find-replace-symbolic")
            .tooltip_text("Replace (Ctrl+H)")
            .build();

        let find_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        find_row.append(&entry);
        find_row.append(&count);
        find_row.append(&arrows);
        find_row.append(&options);
        find_row.append(&replacing);

        let with = gtk::Entry::builder()
            .placeholder_text("Replace with")
            .hexpand(true)
            .build();
        let one = gtk::Button::with_label("Replace");
        one.set_tooltip_text(Some("Replace in this cell, then find the next"));
        let all = gtk::Button::with_label("Replace All");
        all.set_tooltip_text(Some("Every cell in every sheet, in one undo step"));
        let replace_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        replace_row.append(&with);
        replace_row.append(&one);
        replace_row.append(&all);
        let reveal = gtk::Revealer::builder()
            .child(&replace_row)
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .build();
        replacing
            .bind_property("active", &reveal, "reveal-child")
            .sync_create()
            .build();

        let rows = gtk::Box::new(gtk::Orientation::Vertical, 6);
        rows.set_width_request(560);
        rows.append(&find_row);
        rows.append(&reveal);
        let bar = gtk::SearchBar::builder()
            .child(&rows)
            .show_close_button(true)
            .build();
        bar.connect_entry(&entry);

        let find = Rc::new(Self {
            bar,
            entry,
            count,
            replacing,
            with,
            match_case,
            whole_cell,
            app: app.clone(),
            grid: grid.clone(),
            said: RefCell::new(None),
        });

        // Every handler holds the bar weakly: the window owns it, and a closure owning it back
        // would be a cycle nothing ever breaks.
        let on = |find: &Rc<Self>, act: fn(&Self)| {
            let find = Rc::downgrade(find);
            move || {
                if let Some(find) = find.upgrade() {
                    act(&find);
                }
            }
        };
        let here = on(&find, |f| f.go(Towards::Here));
        let forward = on(&find, |f| f.go(Towards::Next));
        let back = on(&find, |f| f.go(Towards::Previous));

        find.entry.connect_search_changed({
            let here = here.clone();
            move |_| here()
        });
        find.entry.connect_activate({
            let forward = forward.clone();
            move |_| forward()
        });
        find.entry.connect_next_match({
            let forward = forward.clone();
            move |_| forward()
        });
        find.entry.connect_previous_match({
            let back = back.clone();
            move |_| back()
        });
        next.connect_clicked({
            let forward = forward.clone();
            move |_| forward()
        });
        previous.connect_clicked({
            let back = back.clone();
            move |_| back()
        });
        // A narrower or wider search is a different set of hits, so it is searched again.
        find.match_case.connect_toggled({
            let here = here.clone();
            move |_| here()
        });
        find.whole_cell.connect_toggled(move |_| here());
        one.connect_clicked({
            let one = on(&find, |f| f.replace_one());
            move |_| one()
        });
        find.with.connect_activate({
            let one = on(&find, |f| f.replace_one());
            move |_| one()
        });
        all.connect_clicked({
            let all = on(&find, |f| f.replace_all());
            move |_| all()
        });

        // Shift+Enter is not one of the entry's own bindings, and it is the key a hand already
        // holding Enter reaches for.
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, state| {
            let enter = matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter);
            match enter && state.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
                true => {
                    back();
                    glib::Propagation::Stop
                }
                false => glib::Propagation::Proceed,
            }
        });
        find.entry.add_controller(keys);

        // Escape, or the close button: the keyboard goes back to the grid, the hit still
        // selected under it.
        let weak = Rc::downgrade(&find);
        find.bar.connect_search_mode_enabled_notify(move |bar| {
            if !bar.is_search_mode()
                && let Some(find) = weak.upgrade()
            {
                find.grid.grab_focus();
            }
        });
        find
    }

    /// Where a finished replace is announced. The window sets it once it exists.
    pub fn connect_said(&self, f: impl Fn(&str) + 'static) {
        *self.said.borrow_mut() = Some(Box::new(f));
    }

    /// Open the bar — with its replace row too when `replacing` (Ctrl+H) — seeded with the
    /// active cell's own text when it is short: what somebody is standing on when they press
    /// Ctrl+F is very often what they want found.
    pub fn open(&self, replacing: bool) {
        // An edit in progress is stored first, as a click elsewhere would store it: the bar is
        // about to move the selection, and an edit must not ride along to another cell.
        if self.grid.is_editing() {
            self.grid.commit(None);
        }
        if self.entry.text().is_empty() {
            let active = self.grid.selection().active;
            let text = self
                .app
                .input_text(self.grid.sheet(), active)
                .unwrap_or_default();
            if !text.is_empty() && text.chars().count() <= 64 && !text.starts_with('=') {
                self.entry.set_text(&text);
            }
        }
        if replacing {
            self.replacing.set_active(true);
        }
        self.bar.set_search_mode(true);
        self.entry.grab_focus();
        self.entry.select_region(0, -1);
    }

    /// Search for `needle` as though it had been typed — for the widget test, which has no
    /// main loop to deliver `search-changed`'s debounce.
    #[cfg(test)]
    pub fn search(&self, needle: &str) {
        self.entry.set_text(needle);
        self.go(Towards::Here);
    }

    /// Enter.
    #[cfg(test)]
    pub fn next(&self) {
        self.go(Towards::Next);
    }

    /// Type into the replace entry and press one of its two buttons.
    #[cfg(test)]
    pub fn replace(&self, with: &str, all: bool) {
        self.with.set_text(with);
        match all {
            true => self.replace_all(),
            false => self.replace_one(),
        }
    }

    #[cfg(test)]
    pub fn count(&self) -> String {
        self.count.text().to_string()
    }

    /// What the entry and the two toggles ask for, over every sheet.
    fn query(&self) -> Search {
        Search {
            needle: self.entry.text().to_string(),
            match_case: self.match_case.is_active(),
            whole_cell: self.whole_cell.is_active(),
            ..Search::default()
        }
    }

    fn here(&self) -> Place {
        (self.grid.sheet(), self.grid.selection().active)
    }

    /// Put the cursor on `place`, changing sheet only when it has to — `set_sheet` resets the
    /// selection, and the page should not flicker for a hit on the sheet already in front.
    fn select(&self, (sheet, pos): Place) {
        if sheet != self.grid.sheet() {
            self.grid.set_sheet(sheet);
        }
        self.grid.set_selection(Selection::at(pos));
    }

    fn go(&self, towards: Towards) {
        let search = self.query();
        self.entry.remove_css_class("error");
        if search.needle.is_empty() {
            self.count.set_text("");
            return;
        }
        let hits: Vec<Place> = match self.app.find(&search) {
            Ok(hits) => hits.into_iter().map(|hit| (hit.sheet, hit.pos)).collect(),
            Err(error) => {
                self.count.set_text(&error.to_string());
                return;
            }
        };
        let Some(index) = step(&hits, self.here(), towards) else {
            self.entry.add_css_class("error");
            self.count.set_text("No results");
            return;
        };
        self.select(hits[index]);
        self.count
            .set_text(&format!("{} of {}", index + 1, hits.len()));
    }

    /// *Replace*: the cell under the cursor, when it is a hit, then on to the next one — and
    /// when it is not, only the step, which is the first press of every find-and-replace
    /// anybody has used: it shows what it is about to change before changing it.
    fn replace_one(&self) {
        let search = self.query();
        if search.needle.is_empty() {
            return;
        }
        let (sheet, pos) = self.here();
        let one = Search {
            sheet: Some(sheet),
            range: Some((pos, pos)),
            ..search
        };
        match self
            .app
            .replace(&one, &self.with.text(), RecalcMode::Document)
        {
            Ok(done) => self.report(&done, false),
            Err(error) => self.grid.report(Notice::Refused(error.to_string())),
        }
        self.go(Towards::Next);
    }

    /// *Replace All*: every hit in every sheet, one undo step, announced with an Undo button.
    fn replace_all(&self) {
        let search = self.query();
        if search.needle.is_empty() {
            return;
        }
        match self
            .app
            .replace(&search, &self.with.text(), RecalcMode::Document)
        {
            Ok(done) => self.report(&done, true),
            Err(error) => self.grid.report(Notice::Refused(error.to_string())),
        }
        self.go(Towards::Here);
    }

    fn report(&self, done: &grind_sheet::find::Replaced, announce: bool) {
        if let Some(recalc) = done.recalc.filter(|r| r.spoiled > 0) {
            self.grid.report(Notice::RecalcSkipped(recalc.spoiled));
        }
        if let Some((hit, reason)) = done.refused.first() {
            // A refusal is always said, even for one cell: the button was pressed and the cell
            // did not change, and silence would read as a bug.
            self.grid.report(Notice::Refused(format!(
                "{} left as it was — the formula would not parse: {reason}",
                hit.address()
            )));
        }
        if announce
            && done.cells > 0
            && let Some(said) = self.said.borrow().as_ref()
        {
            said(&match done.cells {
                1 => "Replaced in 1 cell".to_owned(),
                n => format!("Replaced in {n} cells"),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(sheet: usize, row: u32, col: u32) -> Place {
        (sheet, Pos::new(row, col))
    }

    /// The bar against a real `App` and `Grid`. One `#[test]` for every case, because GTK
    /// lives on the thread that initialised it and a second test would be a second thread.
    #[test]
    fn the_bar_finds_across_sheets_and_replaces_through_the_core() {
        if gtk::init().is_err() {
            eprintln!("no display — skipping the find bar's widget case");
            return;
        }
        let app = Arc::new(App::new());
        let enter = |sheet, row, col, input: &str| {
            app.enter(sheet, Pos::new(row, col), input, RecalcMode::No)
                .expect("enters");
        };
        enter(0, 0, 0, "Apples");
        enter(0, 0, 1, "3");
        enter(0, 1, 1, "=SUM([.B1])");
        app.add_sheet("Notes").expect("a second sheet");
        enter(1, 2, 0, "apples are red");
        let grid = Grid::new(app.clone());
        let find = Find::new(&app, &grid);
        find.open(true);

        find.search("apples");
        assert_eq!((grid.sheet(), grid.selection().active), at(0, 0, 0));
        assert_eq!(find.count(), "1 of 2");
        find.next();
        assert_eq!(
            (grid.sheet(), grid.selection().active),
            at(1, 2, 0),
            "onto the other sheet"
        );
        find.next();
        assert_eq!(
            (grid.sheet(), grid.selection().active),
            at(0, 0, 0),
            "wrapped"
        );

        // Replace changes the cell under the cursor only, then moves on.
        find.replace("Plums", false);
        assert_eq!(app.input_text(0, Pos::new(0, 0)).unwrap(), "Plums");
        assert_eq!(app.input_text(1, Pos::new(2, 0)).unwrap(), "apples are red");
        assert_eq!((grid.sheet(), grid.selection().active), at(1, 2, 0));

        // A formula is searched as the formula bar shows it, and Replace All keeps it one.
        find.search("sum(b1");
        assert_eq!((grid.sheet(), grid.selection().active), at(0, 1, 1));
        find.replace("SUM(B1)*2+SUM(B1", true);
        assert_eq!(
            app.input_text(0, Pos::new(1, 1)).unwrap(),
            "=SUM(B1)*2+SUM(B1)"
        );
        assert_eq!(
            app.get(0, Pos::new(1, 1)).unwrap(),
            grind_sheet::CellValue::Number(9.0)
        );
        assert!(app.undo(), "one undo step");
        assert_eq!(app.input_text(0, Pos::new(1, 1)).unwrap(), "=SUM(B1)");

        find.search("nowhere");
        assert_eq!(find.count(), "No results");
    }
}
