// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **Page Setup…** — the paper, how it lies and the four margins (`doc/page-model.md` PM8), set
//! through `App::set_page` as `grind text page-setup` sets them. The papers are ISO 216's A sizes
//! (`doc/pdf-export.md` decision 7); a page of another size is offered as itself.

use std::sync::Arc;

use grind_core::page::PageGeometry;
use grind_text::App;
use libadwaita as adw;
use libadwaita::gtk;
use libadwaita::prelude::*;

/// The papers offered, by name.
const PAPERS: [&str; 4] = ["A3", "A4", "A5", "A6"];

/// The page a dialog's choices make: `paper` an index into [`PAPERS`] or `None` to keep `page`'s
/// own size, turned to lie `landscape` or not, with these margins in millimetres.
pub fn chosen(
    page: PageGeometry,
    paper: Option<usize>,
    landscape: bool,
    [top, right, bottom, left]: [f64; 4],
) -> PageGeometry {
    let sized = paper
        .and_then(|index| PAPERS.get(index))
        .and_then(|name| PageGeometry::paper(name))
        .map_or(page, |paper| page.with_paper(paper));
    PageGeometry {
        top,
        right,
        bottom,
        left,
        ..sized.turned(landscape)
    }
}

/// Which of [`PAPERS`] `page` is, either way up.
fn paper_of(page: &PageGeometry) -> Option<usize> {
    let name = page.iso_name()?;
    PAPERS
        .iter()
        .position(|paper| name.eq_ignore_ascii_case(paper))
}

/// Open the dialog over `parent`, set on `app`'s page; `refused` hears a page the core refused.
pub fn present(parent: &impl IsA<gtk::Widget>, app: &Arc<App>, refused: impl Fn(String) + 'static) {
    let page = app.page().unwrap_or_default();
    let dialog = adw::AlertDialog::new(Some("Page Setup"), None);

    let mut names: Vec<String> = PAPERS.iter().map(|name| (*name).to_owned()).collect();
    let own = paper_of(&page);
    if own.is_none() {
        names.push(format!(
            "{} × {} mm",
            page.width.min(page.height).round(),
            page.width.max(page.height).round()
        ));
    }
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let paper = gtk::DropDown::from_strings(&names);
    paper.set_selected(own.unwrap_or(PAPERS.len()) as u32);

    let landscape = gtk::CheckButton::with_label("Landscape");
    landscape.set_active(page.width > page.height);

    let margin = |label: &str, mm: f64| {
        let spin = gtk::SpinButton::with_range(0.0, 100.0, 0.5);
        spin.set_digits(1);
        spin.set_value(mm);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let name = gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .hexpand(true)
            .build();
        row.append(&name);
        row.append(&spin);
        row.append(&gtk::Label::new(Some("mm")));
        (row, spin)
    };
    let (top_row, top) = margin("Top", page.top);
    let (right_row, right) = margin("Right", page.right);
    let (bottom_row, bottom) = margin("Bottom", page.bottom);
    let (left_row, left) = margin("Left", page.left);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let paper_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    paper_row.append(
        &gtk::Label::builder()
            .label("Paper")
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    paper_row.append(&paper);
    body.append(&paper_row);
    body.append(&landscape);
    body.append(&gtk::Label::builder().label("Margins").xalign(0.0).build());
    for row in [&top_row, &right_row, &bottom_row, &left_row] {
        body.append(row);
    }
    dialog.set_extra_child(Some(&body));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("set", "Set");
    dialog.set_response_appearance("set", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("set"));
    dialog.set_close_response("cancel");

    let app = app.clone();
    dialog.connect_response(Some("set"), move |_, _| {
        let index = paper.selected() as usize;
        let chosen = chosen(
            page,
            (index < PAPERS.len()).then_some(index),
            landscape.is_active(),
            [top.value(), right.value(), bottom.value(), left.value()],
        );
        if !chosen.has_room() {
            refused("The margins leave no room on the page.".to_owned());
            return;
        }
        if let Err(error) = app.set_page(chosen) {
            refused(error.to_string());
        }
    });
    dialog.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paper_is_turned_as_asked_and_keeps_the_margins_given() {
        let page = PageGeometry::a4();
        let a5 = chosen(page, Some(2), true, [15.0, 15.0, 15.0, 15.0]);
        assert_eq!((a5.width, a5.height), (210.0, 148.0));
        assert_eq!((a5.top, a5.left), (15.0, 15.0));
        // No paper chosen: the page's own size, turned upright.
        let custom = PageGeometry {
            width: 300.0,
            height: 200.0,
            ..page
        };
        let upright = chosen(custom, None, false, [20.0, 20.0, 20.0, 20.0]);
        assert_eq!((upright.width, upright.height), (200.0, 300.0));
    }

    #[test]
    fn an_iso_page_is_found_among_the_papers_either_way_up() {
        assert_eq!(paper_of(&PageGeometry::a4()), Some(1));
        assert_eq!(paper_of(&PageGeometry::a4().landscape()), Some(1));
        assert_eq!(
            paper_of(&PageGeometry {
                width: 215.9,
                height: 279.4,
                ..PageGeometry::a4()
            }),
            None
        );
    }
}
