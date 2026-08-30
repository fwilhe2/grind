// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **What the document says about itself** — `grind text lint` with a list in front of it
//! (`doc/dsl.md` §4.3, D6).
//!
//! `ui_sheet_gtk/src/lint.rs` is this file with `a1` where this one has `loc`, and the two are
//! copies for `code.rs`'s reason: there is no crate a widget both GTK shells could use, since
//! `grind-core` may not hold GTK types and neither application crate may depend on the other
//! (R8). The `ponytail` there is this one — a `grind-gtk` crate when a third GTK shell or a
//! third copy appears.
//!
//! What is *not* copied is anything about the rules. A diagnostic arrives with its severity, its
//! message and an address `loc::parse` takes, so this file decides an icon, a subtitle and a
//! jump — and a shell that got all three wrong still could not make the linter disagree with
//! `grind lint`.

use std::rc::Rc;
use std::sync::Arc;

use grind_text::App;
use grind_text::lint::{Diagnostic, Options, Report, Severity};
use libadwaita as adw;
use libadwaita::gtk;
use libadwaita::prelude::*;

use gtk::glib;

/// The icon a severity is drawn with — Adwaita's own three, so a theme change carries them.
pub fn icon(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "dialog-error-symbolic",
        Severity::Warning => "dialog-warning-symbolic",
        Severity::Hint => "dialog-information-symbolic",
    }
}

/// The line under a finding: where it is, and which rule said so — the id included, because it
/// is the word `grind text lint --off <rule>` takes.
pub fn detail(diagnostic: &Diagnostic) -> String {
    match diagnostic.at.is_empty() {
        true => diagnostic.rule.to_owned(),
        false => format!("{} · {}", diagnostic.at, diagnostic.rule),
    }
}

/// The tally under the header — the sentence the CLI prints, in a window.
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

/// Show the findings. `go_to` is the shell's own jump — the caller passes it because putting the
/// caret somewhere is `Doc`'s business and this file has no view.
pub fn present(
    window: &adw::ApplicationWindow,
    app: &Arc<App>,
    go_to: impl Fn(&str) + Clone + 'static,
) {
    dialog(app, go_to).present(Some(window));
}

/// Build it, without showing it.
///
/// Split from [`present`] so the widget harness can build one: `view.rs`'s `the_widget` is the
/// one place in this crate that has a display and the thread GTK was initialised on, and a
/// dialog that panics on its first `add_prefix` is exactly the failure a table of pure functions
/// cannot catch.
pub fn dialog(app: &Arc<App>, go_to: impl Fn(&str) + Clone + 'static) -> adw::Dialog {
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
        .content_width(520)
        .content_height(520)
        .build();

    let refresh: Rc<dyn Fn()> = {
        let (list, tally, hints) = (list.clone(), tally.clone(), hints.clone());
        let (app, dialog) = (app.clone(), dialog.clone());
        Rc::new(move || {
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
                    dialog,
                    #[strong(rename_to = address)]
                    diagnostic.at,
                    #[strong(rename_to = go_to)]
                    go_to,
                    move |_| {
                        go_to(&address);
                        dialog.close();
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
    dialog
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostic(severity: Severity, at: &str) -> Diagnostic {
        Diagnostic {
            rule: "heading-skip",
            severity,
            at: at.to_owned(),
            message: "heading level 3 follows level 1".to_owned(),
        }
    }

    /// The tables, which are what can drift from the core's vocabulary. No display needed.
    #[test]
    fn every_severity_has_an_icon_of_its_own() {
        let icons: Vec<&str> = [Severity::Error, Severity::Warning, Severity::Hint]
            .into_iter()
            .map(icon)
            .collect();
        assert!(icons.iter().all(|name| name.ends_with("-symbolic")));
        assert!(icons[0] != icons[1] && icons[1] != icons[2] && icons[0] != icons[2]);
    }

    /// A text document's addresses are `loc`'s, and a row shows one a user could type into the
    /// go-to box — which is the same string `grind text lint` prints.
    #[test]
    fn a_row_shows_an_address_the_go_to_box_would_take() {
        assert_eq!(
            detail(&diagnostic(Severity::Warning, "p12")),
            "p12 · heading-skip"
        );
        assert_eq!(detail(&diagnostic(Severity::Warning, "")), "heading-skip");
    }

    #[test]
    fn the_tally_counts_each_severity_and_says_so_when_there_is_nothing() {
        let mut report = Report::default();
        assert_eq!(summary(&report), "No problems found");
        report.push(diagnostic(Severity::Error, "p1"));
        report.push(diagnostic(Severity::Hint, "p2"));
        assert_eq!(summary(&report), "1 error, 1 hint");
    }
}
