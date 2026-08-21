// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The autocomplete popover, and the pure part of deciding what to complete.
//!
//! What it offers comes from [`funcs::catalog`] — the spec's own signature and summary for
//! every function this build implements — and from the document's defined names. Neither
//! list is written here: a shell that kept its own would offer a function the evaluator does
//! not have, which is a promise nothing keeps.
//!
//! The popover does **not** autohide, and that is deliberate: an autohiding popover takes
//! the input grab with it, and the whole point is that typing carries on into the editor
//! underneath while the list narrows.

use std::cell::{Cell, RefCell};
use std::ops::Range;

use libadwaita::gtk;
use libadwaita::prelude::*;

use sheet_core::formula::{friendly, funcs};

use crate::geom::Rect;

/// One offer in the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// What gets inserted, `(` included for a function.
    pub insert: String,
    pub name: String,
    pub detail: String,
}

/// The identifier being typed at `caret`, if it is one an offer could replace.
///
/// A run of name characters that starts where a *function* could start — after `=`, `(`,
/// `;` or an operator — and is not already a call. Pure, and the reason the popover has no
/// opinion of its own about what a word is.
pub fn prefix_at(text: &str, caret: usize) -> Option<Range<usize>> {
    if !text.starts_with('=') {
        return None;
    }
    let caret = caret.min(text.len());
    let start = text[..caret]
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_' || *c == '.')
        .last()
        .map(|(i, _)| i)?;
    if start == 0 {
        return None;
    }
    // Already a call: `SUM(` is finished business, and so is `A1` — a cell address is not a
    // function name, and offering to turn one into a call would be wrong twice over.
    if text[caret..].starts_with('(') {
        return None;
    }
    let before = text[..start].trim_end().chars().next_back()?;
    "=(;+-*/^&<>:,".contains(before).then_some(start..caret)
}

/// Everything worth offering for `prefix`: functions first, then the document's own names.
pub fn candidates(prefix: &str, names: &[String]) -> Vec<Candidate> {
    let upper = prefix.to_uppercase();
    let functions = funcs::catalog()
        .iter()
        .filter(|info| info.name.starts_with(&upper))
        .map(|info| Candidate {
            insert: format!("{}(", info.name),
            name: info.name.to_owned(),
            detail: info.brief.to_owned(),
        });
    let named = names
        .iter()
        .filter(|name| name.to_uppercase().starts_with(&upper))
        .map(|name| Candidate {
            insert: name.clone(),
            name: name.clone(),
            detail: "defined name".to_owned(),
        });
    functions.chain(named).collect()
}

/// The signature hint for the call the caret is in, as Pango markup with the current
/// argument in bold.
///
/// Two spellings of the same signature, `friendly` picking between them: the spec's own
/// `Syntax:` line, types and all, or [`friendly::signature`]'s plain-English one. The
/// friendly spelling is the same vocabulary [`friendly::explain`] labels a finished formula
/// with, so what a user reads while typing and what they read afterwards agree.
///
/// A repeating parameter is the last one however many arguments follow it, which is why the
/// emphasised index is clamped rather than dropped.
pub fn signature_markup(name: &str, argument: usize, friendly: bool) -> Option<String> {
    let (head, parts) = match friendly {
        true => {
            let (head, labels) = friendly::signature(name)?;
            (head, labels)
        }
        false => {
            let info = funcs::catalog()
                .iter()
                .find(|info| info.name.eq_ignore_ascii_case(name))?;
            let (head, rest) = info.signature.split_once('(')?;
            let rest = rest.strip_suffix(')').unwrap_or(rest);
            (
                head.to_owned(),
                rest.split(';').map(str::to_owned).collect(),
            )
        }
    };
    let last = parts.len().saturating_sub(1);
    let separator = match friendly {
        true => "; ",
        false => ";",
    };
    let parts: Vec<String> = parts
        .iter()
        .enumerate()
        .map(|(i, part)| match i == argument.min(last) {
            true => format!("<b>{}</b>", glib_escape(part)),
            false => glib_escape(part),
        })
        .collect();
    Some(format!("{}({})", glib_escape(&head), parts.join(separator)))
}

fn glib_escape(text: &str) -> String {
    gtk::glib::markup_escape_text(text).to_string()
}

/// The popover itself: a list of candidates under the cell being edited.
pub struct Completion {
    popover: gtk::Popover,
    list: gtk::ListBox,
    offers: RefCell<Vec<Candidate>>,
    /// The text range an accepted offer replaces.
    span: RefCell<Range<usize>>,
    chosen: Cell<usize>,
}

/// How many offers are shown. A list longer than this is a list nobody reads, and the
/// prefix is one keystroke from narrowing it.
const MAX_OFFERS: usize = 8;

impl Completion {
    pub fn new(parent: &impl IsA<gtk::Widget>) -> Self {
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .propagate_natural_height(true)
            .propagate_natural_width(true)
            .max_content_height(240)
            .build();
        let popover = gtk::Popover::builder()
            .child(&scroller)
            .autohide(false)
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .build();
        popover.set_parent(parent);
        popover.add_css_class("menu");
        Self {
            popover,
            list,
            offers: RefCell::new(Vec::new()),
            span: RefCell::new(0..0),
            chosen: Cell::new(0),
        }
    }

    /// A popover has to be unparented before its parent goes away.
    pub fn dispose(&self) {
        self.popover.unparent();
    }

    pub fn is_visible(&self) -> bool {
        self.popover.is_visible()
    }

    pub fn hide(&self) {
        self.popover.popdown();
    }

    /// Recompute from the text, and show or hide accordingly. `at` is where the editor is,
    /// so the list appears under what is being typed.
    pub fn update(&self, text: &str, caret: usize, names: &[String], at: Rect) {
        let Some(span) = prefix_at(text, caret) else {
            return self.hide();
        };
        let offers = candidates(&text[span.clone()], names);
        if offers.is_empty() {
            return self.hide();
        }
        // A single offer that is already typed out in full is not an offer.
        if offers.len() == 1 && offers[0].name.eq_ignore_ascii_case(&text[span.clone()]) {
            return self.hide();
        }

        while let Some(row) = self.list.first_child() {
            self.list.remove(&row);
        }
        for offer in offers.iter().take(MAX_OFFERS) {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            let name = gtk::Label::new(Some(&offer.name));
            name.add_css_class("heading");
            name.set_xalign(0.0);
            let detail = gtk::Label::new(Some(&offer.detail));
            detail.add_css_class("dim-label");
            detail.set_xalign(0.0);
            detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
            detail.set_max_width_chars(48);
            row.append(&name);
            row.append(&detail);
            row.set_margin_start(6);
            row.set_margin_end(6);
            self.list.append(&row);
        }
        self.offers
            .replace(offers.into_iter().take(MAX_OFFERS).collect());
        *self.span.borrow_mut() = span;
        self.choose(0);
        self.popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
            at.x as i32,
            at.y as i32,
            at.w as i32,
            at.h as i32,
        )));
        self.popover.popup();
    }

    /// Up and down the list, wrapping — a list this short is quicker to wrap than to stop.
    pub fn step(&self, delta: i32) {
        let count = self.offers.borrow().len() as i32;
        if count == 0 {
            return;
        }
        let next = (self.chosen.get() as i32 + delta).rem_euclid(count);
        self.choose(next as usize);
    }

    fn choose(&self, index: usize) {
        self.chosen.set(index);
        if let Some(row) = self.list.row_at_index(index as i32) {
            self.list.select_row(Some(&row));
        }
    }

    /// What accepting the highlighted offer replaces, and with what.
    pub fn accept(&self) -> Option<(Range<usize>, String)> {
        let offers = self.offers.borrow();
        let offer = offers.get(self.chosen.get())?;
        let replacement = offer.insert.clone();
        let span = self.span.borrow().clone();
        drop(offers);
        self.hide();
        Some((span, replacement))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefix_is_a_word_where_a_function_could_start() {
        assert_eq!(prefix_at("=SU", 3), Some(1..3));
        assert_eq!(prefix_at("=SUM(AV", 7), Some(5..7));
        assert_eq!(prefix_at("=1+VLO", 6), Some(3..6));
        // Not a place a function can start, or not a word at all.
        assert_eq!(prefix_at("=SUM(B2", 7), Some(5..7)); // a word, and B2 is offerable
        assert_eq!(prefix_at("SUM", 3), None); // not a formula
        assert_eq!(prefix_at("=SUM(", 5), None); // nothing typed yet
        assert_eq!(prefix_at("=SUM(1;2)", 9), None);
    }

    #[test]
    fn candidates_come_from_the_catalog_and_the_document() {
        let names = vec!["expenses".to_owned(), "excess".to_owned()];
        let offers = candidates("su", &names);
        assert!(offers.iter().any(|c| c.name == "SUM" && c.insert == "SUM("));
        assert!(offers.iter().all(|c| c.name.starts_with("SU")));

        // Names come after functions, and are inserted without a parenthesis.
        let offers = candidates("ex", &names);
        assert_eq!(
            offers.last().map(|c| c.insert.clone()),
            Some("excess".to_owned())
        );
        assert!(offers.iter().any(|c| c.name == "EXP"));
    }

    #[test]
    fn the_signature_hint_bolds_the_argument_the_caret_is_in() {
        let markup = signature_markup("SUM", 0, false).expect("SUM is in the catalog");
        assert!(markup.starts_with("SUM("), "{markup}");
        assert!(markup.contains("<b>"), "{markup}");

        // The second argument of a two-argument function, and the spec's own names.
        let markup = signature_markup("vlookup", 1, false).expect("case does not matter");
        assert!(
            markup.contains("<b>") && markup.contains("Column"),
            "{markup}"
        );
        assert_eq!(signature_markup("NOSUCHFUNCTION", 0, false), None);
    }

    #[test]
    fn the_friendly_hint_reads_in_the_names_the_explanation_uses() {
        let markup = signature_markup("RATE", 1, true).expect("RATE is in the catalog");
        assert!(markup.starts_with("Interest Rate("), "{markup}");
        assert!(markup.contains("Number Of Periods"), "{markup}");
        assert!(markup.contains("<b>Payment</b>"), "{markup}");

        // Past the last parameter of a repeating one is still that parameter.
        let markup = signature_markup("SUM", 7, true).expect("SUM is in the catalog");
        assert_eq!(markup, "Sum(<b>Number\u{2026}</b>)");
    }
}
