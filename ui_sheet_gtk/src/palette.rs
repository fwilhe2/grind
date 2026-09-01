// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **The command palette — where a verb goes when it is not worth a button.**
//!
//! This window's answer to the growth problem `doc/sheet-shell.md`'s "Four surfaces" section
//! names: every feature is a verb, a toolbar with no membership rule absorbs every verb, and
//! so the toolbar grows without bound. The palette is the surface that is *allowed* to grow.
//! Ctrl+K, type three letters, Enter.
//!
//! **It is a view over the window's own action table and nothing else.** `main.rs`'s
//! [`crate::Verb`] list is where a `win.` action, its accelerator, its menu entry and its
//! shortcuts-window row all already come from; this adds a fourth reader of that one table
//! rather than a second table to keep in step. That is what makes the growth rule
//! *structural* rather than a habit: there is no way to add a verb to this window without it
//! appearing here, because there is no second place to add one.
//!
//! Two things it deliberately is not:
//!
//! * **Not a go-to box.** `grind-web`'s palette also jumps to an address, a sheet or a defined
//!   name, because a browser tab has no other place to put one (`doc/web-shell.md`). This
//!   window has the formula bar's name box, which is where every spreadsheet's user already
//!   looks, and two boxes that both take `B12` is one too many.
//! * **Not a launcher for things a pointer wants.** Bold is on the format bar because the
//!   format bar *shows whether the cell is bold*; the palette can only run it.
//!
//! The ranking is [`grind_core::search::score`] — the same function `ui_web`'s palette ranks
//! with, in the core for exactly that reason.

use std::rc::Rc;

use libadwaita as adw;
use libadwaita::gtk;
use libadwaita::prelude::*;

use gtk::glib;

/// One row of the palette: a `win.` action, spelled for a reader.
///
/// Borrowed from the window's table rather than owned, so a row cannot say something the
/// action table does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    /// The action's name, without the `win.` prefix.
    pub name: &'static str,
    pub title: &'static str,
    pub group: &'static str,
    /// The accelerators, as GTK spells them. Shown in the reader's spelling by [`keys`].
    pub accels: &'static [&'static str],
}

/// A row that matched, and which of its title's characters did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub row: Row,
    /// Character positions in `row.title`, for the emphasis the list draws.
    pub hits: Vec<usize>,
}

/// The rows matching `needle`, best first — or, for an empty query, every row in the order
/// the window declares them, which groups them by what they are about.
///
/// The group is part of the haystack, so *view* finds the readings even though none of their
/// titles contains the word.
pub fn rank(rows: &[Row], needle: &str) -> Vec<Match> {
    let needle = needle.trim();
    if needle.is_empty() {
        return rows
            .iter()
            .map(|row| Match {
                row: *row,
                hits: Vec::new(),
            })
            .collect();
    }
    let mut scored: Vec<(i32, usize, Match)> = rows
        .iter()
        .enumerate()
        .filter_map(|(order, row)| {
            let haystack = format!("{} {}", row.title, row.group);
            let (score, hits) = grind_core::search::score(&haystack, needle)?;
            // Only the hits inside the title can be drawn; the group's are off the end of it.
            let hits = hits
                .into_iter()
                .filter(|at| *at < row.title.chars().count())
                .collect();
            Some((score, order, Match { row: *row, hits }))
        })
        .collect();
    // Declaration order breaks a tie, so the list is stable as a fourth letter is typed.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, found)| found).collect()
}

/// An accelerator list as a reader spells it — `Ctrl+Shift+F`, and nothing at all when the
/// verb has no key of its own, which is most of them.
///
/// The first accelerator only: `redo` has two on purpose (`doc/sheet-shell.md`), and a row
/// that lists both is teaching a choice nobody has to make.
pub fn keys(accels: &[&str]) -> String {
    let Some(accel) = accels.first() else {
        return String::new();
    };
    accel
        .replace("<Control>", "Ctrl+")
        .replace("<Shift>", "Shift+")
        .replace("<Alt>", "Alt+")
        .replace("KP_", "")
        .replace("plus", "+")
        .replace("minus", "−")
        .replace("equal", "=")
        .replace("question", "?")
}

/// The title with its matched characters emphasised, as Pango markup.
///
/// Escaped **after** slicing and per piece, because markup inserted before escaping would be
/// escaped along with the title — and a title with an `&` in it is a document's, not ours,
/// so it will happen.
fn markup(title: &str, hits: &[usize]) -> String {
    let mut out = String::new();
    for (at, character) in title.chars().enumerate() {
        let piece = glib::markup_escape_text(&character.to_string());
        match hits.contains(&at) {
            true => out.push_str(&format!("<b>{piece}</b>")),
            false => out.push_str(&piece),
        }
    }
    out
}

/// Open the palette over `window`, running whichever row is chosen.
///
/// Every row activates a `win.` action, so the palette owns no capability of its own — the
/// same rule the tool strip it replaces was held to, and the reason this file has no `App` in
/// it anywhere.
pub fn present(window: &adw::ApplicationWindow, rows: &'static [Row]) {
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Find a command")
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .build();
    list.add_css_class("boxed-list");
    let empty = gtk::Label::builder().label("No command matches").build();
    empty.add_css_class("dim-label");

    let dialog = adw::Dialog::builder()
        .title("Find a Command")
        .content_width(520)
        .content_height(480)
        .build();

    // Running one is the same three steps however it was chosen — clicked, or Enter on the
    // first row — so it exists once and both paths call it.
    let run: Rc<dyn Fn(&str)> = {
        let (window, dialog) = (window.clone(), dialog.clone());
        Rc::new(move |name: &str| {
            dialog.close();
            // Activated on the window rather than called: the window already owns every one
            // of these, and going through the action is what keeps this file incapable of
            // doing anything the menu and the keyboard cannot.
            WidgetExt::activate_action(&window, &format!("win.{name}"), None).ok();
        })
    };

    // What is currently listed, in the order it is listed — the row widget carries no name of
    // its own, so a chosen row is looked up by its index here. `GObject` user data would do it
    // too and needs `unsafe`; a `Vec` in step with the list does not.
    let shown: Rc<std::cell::RefCell<Vec<&'static str>>> = Rc::default();

    let refresh: Rc<dyn Fn()> = {
        let (list, empty, search) = (list.clone(), empty.clone(), search.clone());
        let shown = shown.clone();
        Rc::new(move || {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            let found = rank(rows, &search.text());
            for entry in &found {
                let title = gtk::Label::builder()
                    .use_markup(true)
                    .label(markup(entry.row.title, &entry.hits))
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build();
                let group = gtk::Label::new(Some(entry.row.group));
                group.add_css_class("dim-label");
                let accelerator = gtk::Label::new(Some(&keys(entry.row.accels)));
                accelerator.add_css_class("dim-label");
                accelerator.add_css_class("numeric");

                let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                line.set_margin_top(8);
                line.set_margin_bottom(8);
                line.set_margin_start(12);
                line.set_margin_end(12);
                for widget in [
                    title.upcast_ref::<gtk::Widget>(),
                    group.upcast_ref(),
                    accelerator.upcast_ref(),
                ] {
                    line.append(widget);
                }
                list.append(&gtk::ListBoxRow::builder().child(&line).build());
            }
            shown.replace(found.iter().map(|entry| entry.row.name).collect());
            list.set_visible(!found.is_empty());
            empty.set_visible(found.is_empty());
            // The first row is what Enter runs, so it is the one that looks chosen.
            if let Some(first) = list.row_at_index(0) {
                list.select_row(Some(&first));
            }
        })
    };
    refresh();
    search.connect_search_changed(glib::clone!(
        #[strong]
        refresh,
        move |_| refresh()
    ));

    list.connect_row_activated(glib::clone!(
        #[strong]
        run,
        #[strong]
        shown,
        move |_, row| {
            if let Some(name) = shown.borrow().get(row.index().max(0) as usize) {
                run(name);
            }
        }
    ));
    // Enter in the search entry runs whatever is selected, which is the whole point of
    // typing three letters — the pointer never has to arrive.
    search.connect_activate(glib::clone!(
        #[weak]
        list,
        #[strong]
        run,
        #[strong]
        shown,
        move |_| {
            let Some(row) = list.selected_row() else {
                return;
            };
            if let Some(name) = shown.borrow().get(row.index().max(0) as usize) {
                run(name);
            }
        }
    ));
    // Down out of the entry moves through the list without losing what was typed — the one
    // key a search-over-a-list needs and does not get for free.
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(glib::clone!(
        #[weak]
        list,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, _| {
            let step = match key {
                gtk::gdk::Key::Down => 1,
                gtk::gdk::Key::Up => -1,
                _ => return glib::Propagation::Proceed,
            };
            let at = list.selected_row().map_or(0, |row| row.index() + step);
            if let Some(row) = list.row_at_index(at.max(0)) {
                list.select_row(Some(&row));
                row.grab_focus();
            }
            glib::Propagation::Stop
        }
    ));
    search.add_controller(keys);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    for widget in [
        search.upcast_ref::<gtk::Widget>(),
        empty.upcast_ref(),
        list.upcast_ref(),
    ] {
        content.append(widget);
    }
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&content)
        .build();
    let view = adw::ToolbarView::builder().content(&scroller).build();
    view.add_top_bar(&adw::HeaderBar::new());
    dialog.set_child(Some(&view));
    dialog.present(Some(window));
    search.grab_focus();
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROWS: [Row; 4] = [
        Row {
            name: "recalc",
            title: "Recalculate",
            group: "Document",
            accels: &["F9"],
        },
        Row {
            name: "lint",
            title: "Check Document",
            group: "Document",
            accels: &["F8"],
        },
        Row {
            name: "show-roles",
            title: "Show what each cell is",
            group: "View",
            accels: &["<Control><Shift>r"],
        },
        Row {
            name: "names",
            title: "Names…",
            group: "Document",
            accels: &[],
        },
    ];

    /// Nothing typed shows everything, in the window's own declaration order — the palette is
    /// the whole vocabulary, so an empty query is a list to read rather than a guess.
    #[test]
    fn an_empty_query_shows_every_verb_in_order() {
        let shown = rank(&ROWS, "");
        assert_eq!(shown.len(), ROWS.len());
        assert_eq!(shown[0].row.name, "recalc");
        assert!(shown.iter().all(|found| found.hits.is_empty()));
    }

    #[test]
    fn three_letters_find_the_verb() {
        assert_eq!(rank(&ROWS, "rec")[0].row.name, "recalc");
        assert_eq!(rank(&ROWS, "check")[0].row.name, "lint");
        assert!(rank(&ROWS, "zzz").is_empty());
    }

    /// The group is searchable, which is what makes the readings findable: not one of their
    /// titles contains the word *view*.
    #[test]
    fn a_group_name_finds_its_members() {
        let found = rank(&ROWS, "view");
        assert_eq!(found[0].row.name, "show-roles");
        // And a hit that landed in the group is dropped before the title is sliced by it.
        assert!(
            found[0]
                .hits
                .iter()
                .all(|at| *at < found[0].row.title.chars().count())
        );
    }

    /// The palette's rows are named by `win.` action, and an action that two rows both name
    /// is one the reader cannot choose between.
    #[test]
    fn no_two_rows_name_the_same_action() {
        let mut names: Vec<&str> = ROWS.iter().map(|row| row.name).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count);
    }

    #[test]
    fn an_accelerator_is_shown_the_way_a_reader_says_it() {
        assert_eq!(keys(&["<Control><Shift>f"]), "Ctrl+Shift+f");
        assert_eq!(keys(&["F9"]), "F9");
        assert_eq!(keys(&[]), "");
        // Two spellings of one key: the first is the one to teach.
        assert_eq!(keys(&["<Control><Shift>z", "<Control>y"]), "Ctrl+Shift+z");
    }

    /// A title's own characters are escaped and the emphasis is not — the one way this can be
    /// wrong and still look right until a document supplies an `&`.
    #[test]
    fn the_emphasis_is_markup_and_the_title_is_not() {
        assert_eq!(markup("A&B", &[0]), "<b>A</b>&amp;B");
        assert_eq!(markup("Bold", &[]), "Bold");
    }
}
