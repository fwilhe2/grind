// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **What the document says about itself** — `grind sheet lint` with a list in front of it. The
//! rules, severities, messages and addresses are the core's; this is a dialog and a jump. A dialog
//! rather than a docked pane, like the calculations explorer: the list is consulted, acted on and
//! closed, and activating a row closes it. A diagnostic's address is a string a person could type
//! at the CLI, resolved sheet name first, since a chart's finding names a sheet and `Sheet1` is
//! also a cell address.

use std::sync::Arc;

use grind_sheet::lint::{Diagnostic, Options, Report, Severity};
use grind_sheet::{App, a1};
use libadwaita as adw;
use libadwaita::gtk;
use libadwaita::prelude::*;

use crate::grid::Grid;
use crate::keymap;
use gtk::glib;

/// The icon a severity is drawn with — Adwaita's own three, so a theme change carries them.
pub fn icon(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "dialog-error-symbolic",
        Severity::Warning => "dialog-warning-symbolic",
        Severity::Hint => "dialog-information-symbolic",
    }
}

/// The line under a finding: where, and which rule — shown because it is what `grind sheet lint
/// --off <rule>` takes.
pub fn detail(diagnostic: &Diagnostic) -> String {
    match diagnostic.at.is_empty() {
        true => diagnostic.rule.to_owned(),
        false => format!("{} · {}", diagnostic.at, diagnostic.rule),
    }
}

/// The tally under the header — the same sentence the CLI prints, in a window.
pub fn summary(report: &Report) -> String {
    if report.is_empty() {
        return "No problems found".to_owned();
    }
    let count = |severity| {
        report
            .diagnostics
            .iter()
            .filter(|d| d.severity == severity)
            .count()
    };
    let mut parts = Vec::new();
    for (severity, one, many) in [
        (Severity::Error, "error", "errors"),
        (Severity::Warning, "warning", "warnings"),
        (Severity::Hint, "hint", "hints"),
    ] {
        match count(severity) {
            0 => {}
            1 => parts.push(format!("1 {one}")),
            n => parts.push(format!("{n} {many}")),
        }
    }
    let mut summary = parts.join(", ");
    if report.truncated {
        summary.push_str(" (stopped counting)");
    }
    summary
}

/// Show the findings, with every row a jump to the place it is about.
pub fn present(window: &adw::ApplicationWindow, app: &Arc<App>, grid: &Grid) {
    let hints = gtk::ToggleButton::builder()
        .label("Hints")
        .tooltip_text("Include house-style hints, which are off by default")
        .build();
    let tally = gtk::Label::builder().wrap(true).xalign(0.0).build();
    tally.add_css_class("dim-label");
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list.add_css_class("boxed-list");

    let dialog = adw::Dialog::builder()
        .title("Check Document")
        .content_width(560)
        .content_height(520)
        .build();

    let refresh: std::rc::Rc<dyn Fn()> = {
        let (list, tally, hints) = (list.clone(), tally.clone(), hints.clone());
        let (app, grid, dialog) = (app.clone(), grid.clone(), dialog.clone());
        std::rc::Rc::new(move || {
            while let Some(row) = list.first_child() {
                list.remove(&row);
            }
            let report = app.lint(&Options {
                hints: hints.is_active(),
                off: Vec::new(),
            });
            for diagnostic in &report.diagnostics {
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&diagnostic.message))
                    .subtitle(glib::markup_escape_text(&detail(diagnostic)))
                    .activatable(true)
                    .build();
                row.add_prefix(&gtk::Image::from_icon_name(icon(diagnostic.severity)));
                row.connect_activated(glib::clone!(
                    #[weak]
                    grid,
                    #[weak]
                    dialog,
                    #[strong(rename_to = app)]
                    app,
                    #[strong(rename_to = address)]
                    diagnostic.at,
                    move |_| {
                        if go_to(&app, &grid, &address) {
                            dialog.close();
                        }
                    }
                ));
                list.append(&row);
            }
            tally.set_label(&summary(&report));
        })
    };
    refresh();
    hints.connect_toggled(glib::clone!(
        #[strong]
        refresh,
        move |_| refresh()
    ));

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let header_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    tally.set_hexpand(true);
    header_row.append(&tally);
    header_row.append(&hints);
    content.append(&header_row);
    content.append(&list);

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&content)
        .build();
    let view = adw::ToolbarView::builder().content(&scroller).build();
    view.add_top_bar(&adw::HeaderBar::new());
    dialog.set_child(Some(&view));
    dialog.present(Some(window));
}

/// Select what a diagnostic is about; `false` when the address names nothing the grid can show,
/// and the row then leaves the dialog open. Sheet name first: `Sheet1` also parses as column
/// `SHEET`, row 1.
fn go_to(app: &Arc<App>, grid: &Grid, address: &str) -> bool {
    if address.is_empty() {
        return false;
    }
    if let Ok(sheet) = a1::sheet(app, address) {
        grid.set_sheet(sheet);
        return true;
    }
    match a1::parse(address).and_then(|reference| a1::resolve(app, &reference)) {
        Ok((sheet, start, _end)) => {
            grid.set_sheet(sheet);
            grid.set_selection(keymap::Selection::at(start));
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tables, which can drift from the core's vocabulary; the widget half is exercised by
    /// `ui_text_gtk`'s harness.
    #[test]
    fn every_severity_has_an_icon_of_its_own() {
        let icons: Vec<&str> = [Severity::Error, Severity::Warning, Severity::Hint]
            .into_iter()
            .map(icon)
            .collect();
        assert_eq!(icons.len(), 3);
        assert!(
            icons.iter().all(|name| name.ends_with("-symbolic")),
            "a themed icon, never a literal glyph: {icons:?}"
        );
        assert!(
            icons[0] != icons[1] && icons[1] != icons[2] && icons[0] != icons[2],
            "three severities the eye can tell apart: {icons:?}"
        );
    }

    fn diagnostic(severity: Severity, at: &str) -> Diagnostic {
        Diagnostic {
            rule: "stale-value",
            severity,
            at: at.to_owned(),
            message: "holds 99 and its formula computes 5".to_owned(),
            location: None,
        }
    }

    #[test]
    fn a_row_says_where_it_is_and_which_rule_said_so() {
        assert_eq!(
            detail(&diagnostic(Severity::Warning, "Sheet1.A3")),
            "Sheet1.A3 · stale-value"
        );
        // A finding about the document as a whole has no address and does not print an empty
        // one — `grind_core::lint`'s own rule, kept here so the two renderings agree.
        assert_eq!(detail(&diagnostic(Severity::Warning, "")), "stale-value");
    }

    #[test]
    fn the_tally_counts_each_severity_and_says_so_when_there_is_nothing() {
        let mut report = Report::default();
        assert_eq!(summary(&report), "No problems found");

        report.push(diagnostic(Severity::Error, "A1"));
        report.push(diagnostic(Severity::Warning, "A2"));
        report.push(diagnostic(Severity::Warning, "A3"));
        assert_eq!(summary(&report), "1 error, 2 warnings");

        report.truncated = true;
        assert!(summary(&report).ends_with("(stopped counting)"));
    }
}
