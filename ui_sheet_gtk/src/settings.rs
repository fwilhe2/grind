// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! *Document Settings* — what is true of the document as a whole rather than of any cell in it,
//! which today is one thing: its own locale, how it spells its numbers
//! (`doc/ods-format.md` §5.2, "The document's own language").
//!
//! An `adw::PreferencesDialog`, and applied the moment it changes, which is what the HIG asks of
//! one: there is no OK to forget. Each change is still one undo step in the document
//! ([`App::set_locale`]), so a wrong pick is Ctrl+Z away like any other edit — the dialog is a
//! view of the document, not a copy of it waiting to be written back.
//!
//! The locale row is searchable, and says what it means rather than what it is called: its
//! subtitle is a sample, `1.234.567,89 · 1234,5`, rendered by the core's own renderer — a
//! grouped two-decimal format and a number with no format at all, the two things the locale
//! decides.

use std::rc::Rc;
use std::sync::Arc;

use libadwaita as adw;
use libadwaita::gtk;
use libadwaita::prelude::*;

use grind_sheet::locale::{self, Locale};
use grind_sheet::numfmt::{self, Kind};
use grind_sheet::{App, CellValue};

/// What the locale row offers, in order: no locale, every [`locale::KNOWN`] one by name, and —
/// when the document states one this build has no name for — that one by its tag, last, so the
/// row can show what the document says rather than pretend it says something else.
pub fn choices(current: Option<&Locale>) -> Vec<Option<Locale>> {
    let mut choices: Vec<Option<Locale>> = std::iter::once(None)
        .chain(locale::KNOWN.iter().map(|(tag, _)| Locale::parse(tag)))
        .collect();
    if let Some(current) = current
        && !choices.iter().any(|c| c.as_ref() == Some(current))
    {
        choices.push(Some(current.clone()));
    }
    choices
}

/// How a choice reads in a list: its name, its tag when it has none, and "None" for no locale.
pub fn label(choice: Option<&Locale>) -> String {
    choice.map_or_else(|| "None".to_owned(), label_of)
}

/// How one locale reads in a list: the name a person knows it by, or its tag when this build
/// has no name for it.
pub fn label_of(locale: &Locale) -> String {
    locale
        .name()
        .map_or_else(|| locale.tag(), |name| name.to_owned())
}

/// What a locale does to a number, in one line: a grouped two-decimal figure and a plain one.
pub fn sample(locale: Option<&Locale>) -> String {
    let grouped = numfmt::preset(Kind::Number, 2, true, numfmt::DEFAULT_CURRENCY)
        .in_locale(locale.cloned())
        .render(&CellValue::Number(1_234_567.89), 0);
    format!("{grouped} · {}", numfmt::spell_number(1234.5, locale))
}

/// Open the dialog over `window`.
pub fn present(window: &impl IsA<gtk::Widget>, app: &Arc<App>) {
    let current = app.locale();
    let choices = Rc::new(choices(current.as_ref()));
    let labels: Vec<String> = choices.iter().map(|c| label(c.as_ref())).collect();
    let model = gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>());

    let row = adw::ComboRow::builder()
        .title("Locale")
        .model(&model)
        .enable_search(true)
        .subtitle(sample(current.as_ref()))
        .build();
    // A search needs to know what an item is called, and a `StringList`'s items are
    // `StringObject`s — this is the one expression that says so.
    row.set_expression(Some(gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        gtk::Expression::NONE,
        "string",
    )));
    let selected = choices.iter().position(|c| *c == current).unwrap_or(0);
    row.set_selected(selected as u32);

    let app = app.clone();
    row.connect_selected_notify(move |row| {
        let Some(choice) = choices.get(row.selected() as usize) else {
            return;
        };
        row.set_subtitle(&sample(choice.as_ref()));
        if app.locale() != *choice {
            let _ = app.set_locale(choice.clone());
        }
    });

    let group = adw::PreferencesGroup::builder()
        .title("Numbers")
        .description(
            "How this document spells a number with no format, or with a format that names no \
             locale of its own — and how a number typed into it is read. A format with a locale \
             of its own keeps it.",
        )
        .build();
    group.add(&row);
    let page = adw::PreferencesPage::builder()
        .title("Document")
        .icon_name("x-office-spreadsheet-symbolic")
        .build();
    page.add(&group);
    let dialog = adw::PreferencesDialog::builder()
        .title("Document Settings")
        .search_enabled(false)
        .build();
    dialog.add(&page);
    dialog.present(Some(window));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choices_start_with_none_and_keep_a_locale_the_list_has_no_name_for() {
        let plain = choices(None);
        assert_eq!(plain[0], None);
        assert_eq!(plain.len(), 1 + locale::KNOWN.len());
        let swiss = Locale::parse("de-CH");
        let with = choices(swiss.as_ref());
        assert_eq!(with.last().cloned().flatten(), swiss);
        assert_eq!(label(swiss.as_ref()), "de-CH");
        assert_eq!(label(Locale::parse("de-DE").as_ref()), "German (Germany)");
        assert_eq!(label(None), "None");
        // A known one is not added twice.
        assert_eq!(choices(Locale::parse("fr-FR").as_ref()).len(), plain.len());
    }

    #[test]
    fn the_sample_shows_what_the_locale_does_to_a_number() {
        assert_eq!(sample(None), "1,234,567.89 · 1234.5");
        assert_eq!(
            sample(Locale::parse("de-DE").as_ref()),
            "1.234.567,89 · 1234,5"
        );
        assert_eq!(
            sample(Locale::parse("fr-FR").as_ref()),
            "1\u{a0}234\u{a0}567,89 · 1234,5"
        );
    }
}
