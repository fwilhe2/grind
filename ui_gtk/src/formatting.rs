// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The format strip — M7's formatting UI (doc/gtk-shell.md).
//!
//! **It maps 1:1 onto the core's vocabulary and adds nothing.** Every toggle here is one
//! field of `style::CellStyle`, and the number-format menu offers exactly the parameters
//! `numfmt::preset` takes — which is the milestone's exit criterion: a document formatted
//! from this strip and one formatted by `sheet format` with the same arguments are the same
//! document, because both ask the core for the same `Format`.
//!
//! Three things carry the weight:
//!
//! * **Setting a style replaces it** ([`App::set_style`]), so a bold button is a *read*
//!   through `App::style_at`, one field, and a write. The read is of the **active** cell,
//!   which is what every spreadsheet shows in its toolbar and what makes toggling
//!   predictable over a mixed selection.
//! * **The write is one `Action::Batch`**, so one Ctrl+Z takes back a formatted column.
//! * **The rectangle is the selection clamped to the used extent** ([`Grid::target`]) — a
//!   whole-column selection must not ask for a million entries, and the core refuses one that
//!   does. A refusal becomes a toast rather than nothing (`Notice::Refused`).
//!
//! The strip is a renderer like everything else: it holds no formatting state of its own and
//! re-reads the active cell whenever the selection moves. `updating` guards that refresh,
//! because setting a `GtkToggleButton` programmatically fires the same signal a click does.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use libadwaita::gtk;
use libadwaita::prelude::*;

use gtk::glib;

use sheet_core::locale::Locale;
use sheet_core::numfmt::{self, Kind};
use sheet_core::style::CellStyle;
use sheet_core::App;

use crate::grid::{Grid, Notice};

/// The number-format vocabulary as a menu offers it: [`numfmt::preset`]'s kinds, plus
/// "General" for no format at all and "Date and time" for §4.3.4's date carrying a time.
///
/// The same list `sheet format` takes as its positional argument. It is spelled twice — once
/// per shell — because the CLI's is a `clap::ValueEnum` and clap does not belong in the core;
/// what must not be spelled twice is the *format*, and that comes from `numfmt` on both
/// sides.
const KINDS: [(&str, Option<Kind>); 9] = [
    ("General", None),
    ("Number", Some(Kind::Number)),
    ("Percent", Some(Kind::Percentage)),
    ("Currency", Some(Kind::Currency)),
    ("Date", Some(Kind::Date)),
    ("Date and time", None), // built by `numfmt::datetime_preset`
    ("Time", Some(Kind::Time)),
    ("Boolean", Some(Kind::Boolean)),
    ("Text", Some(Kind::Text)),
];

/// Where "Date and time" sits in [`KINDS`] — the one entry that is not a `Kind`.
const DATETIME: u32 = 5;

/// The strip, and the one thing anything outside it may do: ask it to re-read the cell.
///
/// Two events move what it should show, and only one of them is a selection change: undo,
/// redo and a load change the *cell* under an unmoved selection, so the window's own refresh
/// calls this too. Same shape as [`crate::chrome::Tabs`], for the same reason.
pub struct Strip {
    pub widget: gtk::Box,
    refresh: Box<dyn Fn()>,
}

impl Strip {
    pub fn refresh(&self) {
        (self.refresh)();
    }
}

pub fn strip(grid: &Grid, app: &Arc<App>) -> Rc<Strip> {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    bar.add_css_class("toolbar");

    // Set while the strip is writing into its own widgets, so their signals know a refresh
    // from a click. A flag rather than blocking handlers: there are nine of them and one
    // `bool` is the whole mechanism.
    let updating = Rc::new(Cell::new(false));

    let bold = toggle("format-text-bold-symbolic", "Bold");
    let italic = toggle("format-text-italic-symbolic", "Italic");
    let emphasis = linked(&[&bold, &italic]);

    let left = toggle("format-justify-left-symbolic", "Align Left");
    let center = toggle("format-justify-center-symbolic", "Align Center");
    let right = toggle("format-justify-right-symbolic", "Align Right");
    let align = linked(&[&left, &center, &right]);

    let wrap = toggle("format-justify-fill-symbolic", "Wrap Text");

    let color = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
    color.set_tooltip_text(Some("Text Colour"));
    let background = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
    background.set_tooltip_text(Some("Cell Background"));

    let clear = gtk::Button::from_icon_name("edit-clear-symbolic");
    clear.set_tooltip_text(Some("Clear Formatting"));

    let numbers = gtk::MenuButton::builder()
        .label("123")
        .tooltip_text("Number Format")
        .build();
    let picker = Picker::new();
    numbers.set_popover(Some(&picker.popover));

    for widget in [
        emphasis.upcast_ref::<gtk::Widget>(),
        align.upcast_ref(),
        wrap.upcast_ref(),
        color.upcast_ref(),
        background.upcast_ref(),
        numbers.upcast_ref(),
        clear.upcast_ref(),
    ] {
        bar.append(widget);
    }

    // --- writing ---

    // Each toggle is one field, and every one of them goes through `restyle`, so "read the
    // active cell, change one thing, write the rectangle" exists once.
    let field = |grid: &Grid, app: &Arc<App>, updating: &Rc<Cell<bool>>| {
        let (grid, app, updating) = (grid.clone(), app.clone(), updating.clone());
        move |set: &dyn Fn(&mut CellStyle)| {
            if updating.get() {
                return;
            }
            restyle(&grid, &app, set);
        }
    };

    let apply = field(grid, app, &updating);
    bold.connect_toggled(move |button| {
        let on = button.is_active();
        apply(&|style| style.font_weight = on.then(|| "bold".to_owned()));
    });

    let apply = field(grid, app, &updating);
    italic.connect_toggled(move |button| {
        let on = button.is_active();
        apply(&|style| style.font_style = on.then(|| "italic".to_owned()));
    });

    let apply = field(grid, app, &updating);
    wrap.connect_toggled(move |button| {
        let on = button.is_active();
        apply(&|style| style.wrap = on.then(|| "wrap".to_owned()));
    });

    // §16.5's values are relative to the writing direction, which is why `start`/`end` are
    // stored rather than left/right — the same spelling `sheet style --align` writes.
    for (button, value) in [(&left, "start"), (&center, "center"), (&right, "end")] {
        let apply = field(grid, app, &updating);
        button.connect_toggled(move |button| {
            let on = button.is_active();
            apply(&|style| style.align = on.then(|| value.to_owned()));
        });
    }

    for (button, text) in [(&color, true), (&background, false)] {
        let apply = field(grid, app, &updating);
        button.connect_rgba_notify(move |button| {
            let hex = hex(button.rgba());
            apply(&|style| match text {
                true => style.color = Some(hex.clone()),
                false => style.background = Some(hex.clone()),
            });
        });
    }

    // Clearing is `set_style(None)` and `set_format(None)` — the two "plain again" calls the
    // core already has, and the same pair `sheet style <range>` and `sheet format <range>
    // general` make.
    clear.connect_clicked(glib::clone!(
        #[weak]
        grid,
        #[strong]
        app,
        move |_| {
            let Some((sheet, start, end)) = grid.target() else {
                return;
            };
            if let Err(error) = app.set_style(sheet, start, end, None) {
                grid.report(Notice::Refused(error.to_string()));
                return;
            }
            if let Err(error) = app.set_format(sheet, start, end, None) {
                grid.report(Notice::Refused(error.to_string()));
            }
        }
    ));

    picker.connect_applied(grid, app);

    // --- reading ---

    let refresh = {
        let (app, picker, updating) = (app.clone(), picker.clone(), updating.clone());
        let toggles = (bold, italic, wrap, left, center, right);
        let colors = (color, background);
        let grid = grid.downgrade();
        move || {
            let Some(grid) = grid.upgrade() else { return };
            let pos = grid.selection().active;
            let (style, format) = match (
                app.style_at(grid.sheet(), pos),
                app.format_at(grid.sheet(), pos),
            ) {
                (Ok(style), Ok(format)) => (style.unwrap_or_default(), format),
                _ => return,
            };
            updating.set(true);
            let (bold, italic, wrap, left, center, right) = &toggles;
            bold.set_active(style.font_weight.as_deref() == Some("bold"));
            italic.set_active(matches!(style.font_style.as_deref(), Some("italic" | "oblique")));
            wrap.set_active(style.wrap.as_deref() == Some("wrap"));
            let align = style.align.as_deref();
            left.set_active(matches!(align, Some("start" | "left")));
            center.set_active(align == Some("center"));
            right.set_active(matches!(align, Some("end" | "right")));
            // A cell with no colour of its own shows the **theme's**, not the swatch's own
            // default — a red swatch over an unstyled cell is a claim about the cell.
            let palette = crate::theme::Palette::of(&colors.0);
            for (button, value, unset) in [
                (&colors.0, &style.color, palette.foreground),
                (&colors.1, &style.background, palette.background),
            ] {
                let rgba = value.as_deref().and_then(crate::theme::color).unwrap_or(unset);
                button.set_rgba(&rgba);
            }
            picker.show(format.as_ref());
            updating.set(false);
        }
    };

    let strip = Rc::new(Strip {
        widget: bar,
        refresh: Box::new(refresh),
    });
    strip.refresh();
    grid.connect_selection_changed(glib::clone!(
        #[strong]
        strip,
        move |_| strip.refresh()
    ));

    strip
}

/// Read the active cell's style, change one field, write it over the whole rectangle.
///
/// The read is what makes "bold as well" work: `App::set_style` replaces, deliberately (its
/// docs say so), and this is the read-merge-write its docs promise instead of a merge policy
/// in the core.
fn restyle(grid: &Grid, app: &Arc<App>, set: &dyn Fn(&mut CellStyle)) {
    let Some((sheet, start, end)) = grid.target() else {
        return;
    };
    let mut style = app
        .style_at(sheet, grid.selection().active)
        .ok()
        .flatten()
        .unwrap_or_default();
    set(&mut style);
    // A style that sets nothing *is* no style, and the core spells that `None` — otherwise
    // un-bolding the only styled cell would leave an empty `style:style` behind.
    let style = (!style.is_plain()).then_some(style);
    if let Err(error) = app.set_style(sheet, start, end, style) {
        grid.report(Notice::Refused(error.to_string()));
    }
}

/// The number-format menu: exactly [`numfmt::preset`]'s parameters, and an Apply.
///
/// Applied on a button rather than on every change, because four widgets describe *one*
/// format and applying halfway through would write two formats a user never asked for.
#[derive(Clone)]
struct Picker {
    popover: gtk::Popover,
    kind: gtk::DropDown,
    decimals: gtk::SpinButton,
    grouping: gtk::CheckButton,
    symbol: gtk::Entry,
    locale: gtk::Entry,
    apply: gtk::Button,
    /// Says a document's format is one this vocabulary cannot spell, rather than offering
    /// parameters that would quietly replace it (`Format::is_preset`).
    note: gtk::Label,
}

impl Picker {
    fn new() -> Rc<Self> {
        let labels: Vec<&str> = KINDS.iter().map(|(label, _)| *label).collect();
        let kind = gtk::DropDown::from_strings(&labels);
        let decimals = gtk::SpinButton::with_range(0.0, 10.0, 1.0);
        decimals.set_value(2.0);
        let grouping = gtk::CheckButton::with_label("Group thousands");
        let symbol = gtk::Entry::builder().text("$").max_width_chars(4).build();
        let locale = gtk::Entry::builder()
            .placeholder_text("e.g. de-DE")
            .max_width_chars(8)
            .build();
        let apply = gtk::Button::with_label("Apply");
        apply.add_css_class("suggested-action");
        let note = gtk::Label::builder()
            .label("This cell's format is not one this picker can build")
            .wrap(true)
            .max_width_chars(28)
            .visible(false)
            .build();
        note.add_css_class("dim-label");

        let grid = gtk::Grid::builder()
            .row_spacing(6)
            .column_spacing(8)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        grid.attach(&label("Format"), 0, 0, 1, 1);
        grid.attach(&kind, 1, 0, 1, 1);
        grid.attach(&label("Decimals"), 0, 1, 1, 1);
        grid.attach(&decimals, 1, 1, 1, 1);
        grid.attach(&grouping, 1, 2, 1, 1);
        grid.attach(&label("Symbol"), 0, 3, 1, 1);
        grid.attach(&symbol, 1, 3, 1, 1);
        grid.attach(&label("Locale"), 0, 4, 1, 1);
        grid.attach(&locale, 1, 4, 1, 1);
        grid.attach(&note, 0, 5, 2, 1);
        grid.attach(&apply, 0, 6, 2, 1);

        let picker = Rc::new(Self {
            popover: gtk::Popover::builder().child(&grid).build(),
            kind,
            decimals,
            grouping,
            symbol,
            locale,
            apply,
            note,
        });
        // Digits belong to the numeric families and a symbol to currency; the rest of the
        // menu would be offering parameters the format cannot carry.
        picker.kind.connect_selected_notify(glib::clone!(
            #[strong]
            picker,
            move |_| picker.sensitivity()
        ));
        picker.sensitivity();
        picker
    }

    fn sensitivity(self: &Rc<Self>) {
        let kind = KINDS.get(self.kind.selected() as usize).and_then(|(_, k)| *k);
        let numeric = matches!(
            kind,
            Some(Kind::Number | Kind::Percentage | Kind::Currency)
        );
        self.decimals.set_sensitive(numeric);
        self.grouping.set_sensitive(numeric);
        self.symbol.set_sensitive(kind == Some(Kind::Currency));
        // General is the absence of a format, so nothing else on the menu applies to it.
        let formatted = kind.is_some() || self.kind.selected() == DATETIME;
        self.locale.set_sensitive(formatted);
    }

    /// Show a cell's current format — the picker's whole read half.
    fn show(self: &Rc<Self>, format: Option<&numfmt::Format>) {
        let Some(format) = format else {
            self.kind.set_selected(0);
            self.note.set_visible(false);
            self.sensitivity();
            return;
        };
        let (kind, decimals, grouping, symbol) = format.preset_params();
        let datetime = *format == numfmt::datetime_preset().in_locale(format.locale.clone());
        let selected = match datetime {
            true => DATETIME,
            false => KINDS
                .iter()
                .position(|(_, k)| *k == Some(kind))
                .unwrap_or(0) as u32,
        };
        self.kind.set_selected(selected);
        self.decimals.set_value(f64::from(decimals));
        self.grouping.set_active(grouping);
        if !symbol.is_empty() {
            self.symbol.set_text(&symbol);
        }
        self.locale
            .set_text(&format.locale.as_ref().map(Locale::tag).unwrap_or_default());
        self.note.set_visible(!format.is_preset());
        self.sensitivity();
    }

    /// The write half: build the `Format` the core builds for `sheet format`, and set it.
    fn connect_applied(self: &Rc<Self>, grid: &Grid, app: &Arc<App>) {
        self.apply.connect_clicked(glib::clone!(
            #[strong(rename_to = picker)]
            self,
            #[weak]
            grid,
            #[strong]
            app,
            move |_| {
                picker.popover.popdown();
                let Some((sheet, start, end)) = grid.target() else {
                    return;
                };
                let locale = match picker.locale.text().trim() {
                    "" => None,
                    tag => match Locale::parse(tag) {
                        Some(locale) => Some(locale),
                        // A tag that is not a tag is a typo, not a format: say so and change
                        // nothing, rather than writing an unmarked format.
                        None => {
                            grid.report(Notice::Refused(format!("{tag} is not a locale tag")));
                            return;
                        }
                    },
                };
                let selected = picker.kind.selected();
                let format = match KINDS.get(selected as usize).and_then(|(_, kind)| *kind) {
                    _ if selected == DATETIME => Some(numfmt::datetime_preset()),
                    None => None,
                    Some(kind) => Some(numfmt::preset(
                        kind,
                        picker.decimals.value() as u8,
                        picker.grouping.is_active(),
                        &picker.symbol.text(),
                    )),
                }
                .map(|format| format.in_locale(locale));
                if let Err(error) = app.set_format(sheet, start, end, format) {
                    grid.report(Notice::Refused(error.to_string()));
                }
            }
        ));
    }
}

fn toggle(icon: &str, tooltip: &str) -> gtk::ToggleButton {
    let button = gtk::ToggleButton::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .build();
    button.add_css_class("flat");
    button
}

fn linked(buttons: &[&gtk::ToggleButton]) -> gtk::Box {
    let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    box_.add_css_class("linked");
    for button in buttons {
        box_.append(*button);
    }
    box_
}

fn label(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).build()
}

/// A colour as ODF spells one — `#rrggbb`, which is what `style.rs` keeps verbatim. Alpha is
/// dropped because ODF has nowhere to put it (`fo:background-color` is a colour or
/// `transparent`, §16.5).
fn hex(rgba: gtk::gdk::RGBA) -> String {
    let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        byte(rgba.red()),
        byte(rgba.green()),
        byte(rgba.blue())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sheet_core::style;

    /// The strip's vocabulary is the core's. Both halves of that are checkable without a
    /// display: every kind the menu offers is one `numfmt` builds, and a colour makes the
    /// round trip a document needs it to make.
    #[test]
    fn every_menu_entry_names_a_format_the_core_builds() {
        for (index, (label, kind)) in KINDS.iter().enumerate() {
            match kind {
                Some(kind) => {
                    let format = numfmt::preset(*kind, 2, false, "$");
                    assert_eq!(format.preset_params().0, *kind, "{label}");
                    assert!(format.is_preset(), "{label}");
                }
                // The two entries that are not a `Kind`: General is no format at all, and
                // Date and time has its own constructor.
                None => assert!(
                    index == 0 || index as u32 == DATETIME,
                    "{label} names no format"
                ),
            }
        }
        assert!(numfmt::datetime_preset().is_preset());
    }

    #[test]
    fn a_colour_is_written_the_way_odf_spells_one() {
        assert_eq!(hex(gtk::gdk::RGBA::new(1.0, 1.0, 0.0, 1.0)), "#ffff00");
        // And parses back, which is what the grid does when it paints the cell.
        assert_eq!(
            crate::theme::color("#ffff00"),
            Some(gtk::gdk::RGBA::new(1.0, 1.0, 0.0, 1.0))
        );
        assert_eq!(crate::theme::color("transparent"), None);
    }

    /// Every field the strip writes is one the model carries, and one `sheet style` writes
    /// the same way — the milestone's exit criterion, held by a test rather than by eye.
    #[test]
    fn every_toggle_is_one_field_of_the_core_style() {
        let style = CellStyle {
            font_weight: Some("bold".into()),
            font_style: Some("italic".into()),
            wrap: Some("wrap".into()),
            align: Some("start".into()),
            color: Some("#0000aa".into()),
            background: Some("#ffff00".into()),
            ..CellStyle::default()
        };
        assert!(!style.is_plain());
        // And clearing every one of them is what the core calls plain, which is what makes
        // un-bolding the last styled cell leave nothing behind.
        assert!(CellStyle::default().is_plain());
        assert_eq!(style::EDGES.len(), 4, "borders stay a four-edge array");
    }
}
