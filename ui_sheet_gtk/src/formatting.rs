// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The format strip — M7's formatting UI (doc/sheet-shell.md).
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

use gtk::{gdk, glib};

use grind_sheet::App;
use grind_sheet::locale::Locale;
use grind_sheet::numfmt::{self, Kind};
use grind_sheet::style::{self, CellStyle};

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
    // No `toolbar` class of its own: the strip is what `chrome::format_bar` wraps, and the
    // padding comes from that container — twice would be a taller row than the window wants.
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);

    // Set while the strip is writing into its own widgets, so their signals know a refresh
    // from a click. A flag rather than blocking handlers: there are nine of them and one
    // `bool` is the whole mechanism.
    let updating = Rc::new(Cell::new(false));

    // Each control is one field, and every one of them goes through `restyle`, so "read the
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

    let bold = toggle("format-text-bold-symbolic", "Bold");
    let italic = toggle("format-text-italic-symbolic", "Italic");
    let emphasis = linked(&[&bold, &italic]);

    let left = toggle("format-justify-left-symbolic", "Align Left");
    let center = toggle("format-justify-center-symbolic", "Align Center");
    let right = toggle("format-justify-right-symbolic", "Align Right");
    let align = linked(&[&left, &center, &right]);

    let wrap = toggle("format-justify-fill-symbolic", "Wrap Text");

    // The two colour buttons offer `style::PALETTE` — the palette a document written from
    // either shell uses — with a dialog behind "Custom…" for anything else.
    let color = {
        let apply = field(grid, app, &updating);
        Swatches::new(
            &gtk::Label::new(Some("A")).upcast(),
            "Text Colour",
            move |value| apply(&|style| style.color = value.clone()),
        )
    };
    let background = {
        let apply = field(grid, app, &updating);
        Swatches::new(
            &gtk::Image::from_icon_name("color-select-symbolic").upcast(),
            "Cell Background",
            move |value| apply(&|style| style.background = value.clone()),
        )
    };

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
        color.button.upcast_ref(),
        background.button.upcast_ref(),
        numbers.upcast_ref(),
        clear.upcast_ref(),
    ] {
        bar.append(widget);
    }

    // --- writing ---

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
            italic.set_active(matches!(
                style.font_style.as_deref(),
                Some("italic" | "oblique")
            ));
            wrap.set_active(style.wrap.as_deref() == Some("wrap"));
            let align = style.align.as_deref();
            left.set_active(matches!(align, Some("start" | "left")));
            center.set_active(align == Some("center"));
            right.set_active(matches!(align, Some("end" | "right")));
            // A cell with no colour of its own shows the **theme's**, not a swatch's own
            // default — a red swatch over an unstyled cell is a claim about the cell.
            colors.0.show(style.color.as_deref(), true);
            colors.1.show(style.background.as_deref(), false);
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

/// One colour button: the cell's colour on its face, [`style::PALETTE`] as the choices, and a
/// dialog behind *Custom…* for anything else.
///
/// The palette is a **default, not a limit** — it lives in the core so that `sheet style
/// --color navy` and this button's navy swatch write the same attribute, and the dialog is
/// what keeps arbitrary colours reachable. *Automatic* is the third answer and the one a
/// colour dialog cannot give: remove the attribute, and let the theme decide again.
struct Swatches {
    button: gtk::MenuButton,
    /// The colour drawn on the button's face — the cell's own, or the theme's when the cell
    /// has none. A `Cell` because the draw function reads it on every frame.
    shown: Rc<Cell<gdk::RGBA>>,
    face: gtk::DrawingArea,
}

impl Swatches {
    fn new(head: &gtk::Widget, tooltip: &str, pick: impl Fn(Option<String>) + 'static) -> Rc<Self> {
        let pick = Rc::new(pick);
        let shown = Rc::new(Cell::new(gdk::RGBA::BLACK));
        let face = area(shown.clone(), 16, 4);

        let stack = gtk::Box::new(gtk::Orientation::Vertical, 2);
        stack.set_halign(gtk::Align::Center);
        stack.append(head);
        stack.append(&face);

        let popover = gtk::Popover::new();
        let button = gtk::MenuButton::builder()
            .child(&stack)
            .tooltip_text(tooltip)
            .popover(&popover)
            .build();
        button.add_css_class("flat");

        let choices = palette_grid(&popover, shown.get(), move |v| pick(v));
        popover.set_child(Some(&choices));

        Rc::new(Self {
            button,
            shown,
            face,
        })
    }

    /// Show what the cell has: its own colour, or the theme's when it has none.
    ///
    /// `text` picks *which* of the theme's, because "no colour" means the foreground for text
    /// and the sheet's background for a fill.
    fn show(&self, value: Option<&str>, text: bool) {
        let palette = crate::theme::Palette::of(&self.face);
        let unset = match text {
            true => palette.foreground,
            false => palette.background,
        };
        self.shown
            .set(value.and_then(crate::theme::color).unwrap_or(unset));
        self.face.queue_draw();
    }
}

/// A rectangle that paints one colour — a swatch, with a hairline so that white on white is
/// still a swatch.
fn area(color: Rc<Cell<gdk::RGBA>>, width: i32, height: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(width);
    area.set_content_height(height);
    area.set_draw_func(move |area, cr, w, h| {
        let rgba = color.get();
        let (w, h) = (f64::from(w), f64::from(h));
        cr.set_source_rgba(
            f64::from(rgba.red()),
            f64::from(rgba.green()),
            f64::from(rgba.blue()),
            f64::from(rgba.alpha()),
        );
        cr.rectangle(0.0, 0.0, w, h);
        let _ = cr.fill();
        // The hairline is the theme's foreground, faded — the same trick the grid's lines use,
        // and the reason a white swatch has an edge in both light and dark.
        let edge = crate::theme::with_alpha(crate::theme::Palette::of(area).foreground, 0.3);
        cr.set_source_rgba(
            f64::from(edge.red()),
            f64::from(edge.green()),
            f64::from(edge.blue()),
            f64::from(edge.alpha()),
        );
        cr.set_line_width(1.0);
        cr.rectangle(0.5, 0.5, w - 1.0, h - 1.0);
        let _ = cr.stroke();
    });
    area
}

/// The palette grid plus *Automatic*/*Custom…* every colour picker in this shell offers —
/// [`Swatches::new`]'s own content, and [`crate::grid`]'s chart-mark colour popover, which has
/// no cell to hang a `MenuButton` off of and builds a bare [`gtk::Popover`] around this instead.
/// `shown` seeds *Custom…*'s dialog; `popover` is closed once a choice is made.
pub(crate) fn palette_grid(
    popover: &gtk::Popover,
    shown: gdk::RGBA,
    pick: impl Fn(Option<String>) + 'static,
) -> gtk::Grid {
    let pick = Rc::new(pick);
    let choices = gtk::Grid::builder()
        .row_spacing(4)
        .column_spacing(4)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();
    // Six to a row, which puts the greys on the last one — the palette's own order, so a
    // user reading clrs.cc and a user reading this menu see the same thing.
    for (index, (name, hex)) in style::PALETTE.iter().enumerate() {
        let Some(rgba) = crate::theme::color(hex) else {
            continue;
        };
        let swatch = gtk::Button::builder()
            .child(&area(Rc::new(Cell::new(rgba)), 20, 20))
            .tooltip_text(capitalised(name))
            .build();
        swatch.add_css_class("flat");
        swatch.connect_clicked(glib::clone!(
            #[weak]
            popover,
            #[strong]
            pick,
            move |_| {
                popover.popdown();
                pick(Some((*hex).to_owned()));
            }
        ));
        choices.attach(&swatch, (index % 6) as i32, (index / 6) as i32, 1, 1);
    }

    let automatic = gtk::Button::with_label("Automatic");
    automatic.connect_clicked(glib::clone!(
        #[weak]
        popover,
        #[strong]
        pick,
        move |_| {
            popover.popdown();
            pick(None);
        }
    ));
    choices.attach(&automatic, 0, 3, 3, 1);

    let custom = gtk::Button::with_label("Custom…");
    custom.connect_clicked(glib::clone!(
        #[weak]
        popover,
        #[strong]
        pick,
        move |button| {
            popover.popdown();
            let window = button.root().and_downcast::<gtk::Window>();
            let pick = pick.clone();
            gtk::ColorDialog::new().choose_rgba(
                window.as_ref(),
                Some(&shown),
                gtk::gio::Cancellable::NONE,
                move |chosen| {
                    if let Ok(rgba) = chosen {
                        pick(Some(hex(rgba)));
                    }
                },
            );
        }
    ));
    choices.attach(&custom, 3, 3, 3, 1);
    choices
}

/// A palette name as a menu shows it. The names are ASCII and lower-case by construction
/// (`style::PALETTE`), so this is the whole of the transformation.
fn capitalised(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
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
        let kind = KINDS
            .get(self.kind.selected() as usize)
            .and_then(|(_, k)| *k);
        let numeric = matches!(kind, Some(Kind::Number | Kind::Percentage | Kind::Currency));
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
                    "" => grind_sheet::locale::from_environment(),
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

    /// The swatches are the core's palette, both ways: every hex paints, and every label a
    /// menu shows resolves back to the colour it painted — which is what makes a navy swatch
    /// and `sheet style --color navy` the same attribute.
    #[test]
    fn every_palette_colour_paints_and_its_label_resolves_back() {
        for (name, hex) in style::PALETTE {
            assert!(
                crate::theme::color(hex).is_some(),
                "{name} is {hex}, which does not parse as a colour"
            );
            assert_eq!(
                style::palette(&capitalised(name)),
                Some(hex),
                "the menu's label for {hex} does not resolve back to it"
            );
        }
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
