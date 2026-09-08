// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The formatting bar — every property of a *run* this window can read and write.
//!
//! `ui_sheet_gtk/src/formatting.rs`'s counterpart, one document type over, and it earns its
//! place by the same admission test `doc/sheet-shell.md` sets for that shell's format bar: a
//! control belongs here when it **reads and writes a property of the selection**, and nowhere
//! else. `CharStyle`'s eight fields are therefore the whole bound of this file — there is no
//! room here for a verb, and a verb that wants a home goes in the menu.
//!
//! **Every control reads the document and writes the document, and keeps nothing.** The toggles
//! show what [`grind_text::App::char_style`] says the selection agrees about and write through
//! `set_char_style`; the two swatches and the two drop-downs do exactly the same with the four
//! properties that carry a value rather than a boolean. No widget here holds an opinion about
//! what is bold — there is one such fact and it is in the document.
//!
//! What is testable without a display lives in [`Change`], which is the whole vocabulary of the
//! bar as a pure function over a [`CharStyle`], and in [`sizes`]. The widgets around them need
//! GTK and are exercised by `view.rs`'s one widget harness.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use libadwaita::gtk;
use libadwaita::prelude::*;

use grind_core::style::PALETTE;
use grind_text::CharStyle;
use gtk::{gdk, glib, pango};

/// What one control asks for, as a change to the selection's common formatting.
///
/// A pure enum rather than eight closures, so that "what does the Code button do" is answerable
/// — and answered — with no display: [`Change::apply`] is the whole of it, and its test is the
/// specification of this bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Bold(bool),
    Italic(bool),
    Underline(bool),
    Strike(bool),
    /// The monospace family, which is what the `` `code` `` notation sets — a *family* and not
    /// a fifth boolean, because that is what the document stores (`grind_text::markdown`).
    Code(bool),
    /// `fo:font-family`, verbatim. `None` clears it, which is what "the document's own font"
    /// means: the attribute is absent rather than set to a name.
    Family(Option<String>),
    /// `fo:font-size`, an ODF length such as `14pt`.
    Size(Option<String>),
    Color(Option<String>),
    Highlight(Option<String>),
    /// Every property at once, off. The one-shot the four toggles could only approximate.
    Clear,
}

impl Change {
    /// Lay this change over what the selection already agrees about.
    ///
    /// Everything but [`Change::Clear`] touches exactly one property and leaves the other seven
    /// alone, which is what makes italicising a bold run leave it bold.
    pub fn apply(&self, style: &mut CharStyle) {
        match self {
            Change::Bold(on) => style.set_bold(*on),
            Change::Italic(on) => style.set_italic(*on),
            Change::Underline(on) => style.set_underlined(*on),
            Change::Strike(on) => style.set_struck(*on),
            Change::Code(on) => {
                style.font_family = on.then(|| grind_text::markdown::MONOSPACE.to_owned());
            }
            Change::Family(family) => style.font_family = family.clone(),
            Change::Size(size) => style.font_size = size.clone(),
            Change::Color(color) => style.color = color.clone(),
            Change::Highlight(color) => style.background = color.clone(),
            Change::Clear => *style = CharStyle::default(),
        }
    }
}

/// The sizes the drop-down offers, in points.
///
/// A word processor's usual ladder. It is a **default and not a limit**, the same stance
/// `grind_core::style::PALETTE` takes for colours: `grind text format --size 13pt` writes one
/// that is not on this list, the document keeps it verbatim, and [`Bar::show`] puts it in the
/// list for as long as it is selected.
pub const SIZES: [u32; 12] = [8, 9, 10, 11, 12, 14, 16, 18, 24, 32, 48, 72];

/// The sizes as a document spells them — `"11pt"` — with the document's own size first when it
/// is not one of them, so a selection at `13pt` has something to be selected.
pub fn sizes(current: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = SIZES.iter().map(|pt| format!("{pt}pt")).collect();
    if let Some(current) = current
        && !out.iter().any(|size| size == current)
    {
        out.insert(0, current.to_owned());
    }
    out.insert(0, DEFAULT.to_owned());
    out
}

/// The Monospace toggle's icon.
///
/// Adwaita has no `code-symbolic`, and an icon name the theme has never heard of draws as the
/// missing-image placeholder rather than failing — which is a bug you only find by looking at a
/// screenshot, and this one was found exactly that way. The terminal icon is in Adwaita's own
/// set and reads as "fixed width" to anybody who would reach for this button.
const CODE_ICON: &str = "utilities-terminal-symbolic";

/// What the two drop-downs call "no value set at all". Not a size and not a family: the
/// attribute is absent and the block's own face decides, which is a different thing from
/// setting the same value the face happens to have.
pub const DEFAULT: &str = "Default";

/// One toggle and the [`Change`] it asks for when pressed.
type Toggle = (gtk::ToggleButton, fn(bool) -> Change);

/// Where a control's [`Change`] goes — the window, once [`Bar::connect`] has named one.
type Apply = Rc<dyn Fn(Change)>;

/// The formatting bar's widgets, and the one callback they all reach the document through.
pub struct Bar {
    pub widget: gtk::Box,
    toggles: Vec<Toggle>,
    family: gtk::DropDown,
    families: gtk::StringList,
    size: gtk::DropDown,
    sizes: gtk::StringList,
    color: Rc<Swatch>,
    highlight: Rc<Swatch>,
    /// Guards [`Bar::show`]'s own writes from being read back as a click — without it,
    /// painting the bar's state would immediately rewrite the document it is reporting on.
    /// The same latch `Ui` keeps for the same reason, and it is here as well as there because
    /// a drop-down's `selected` notification arrives from inside `set_selected`.
    updating: Cell<bool>,
    /// Where a control's [`Change`] goes. Empty until [`Bar::connect`] fills it in, because
    /// the bar is built before the window that writes through it exists — and a control
    /// pressed in between does nothing rather than reaching a half-built window.
    apply: RefCell<Option<Apply>>,
}

impl Bar {
    /// Build the bar. Nothing is wired to a document until [`Bar::connect`]; `context` is the
    /// view's own Pango context, which is where the list of font families comes from.
    pub fn new(context: &pango::Context) -> Rc<Self> {
        // The four booleans plus Code, linked into one group the way a toolbar's related
        // toggles are. Code is one of them rather than off with the family drop-down because
        // it is a toggle in the notation and a toggle to the hand: `` `ls -l` `` typed and
        // this button pressed have to be the same edit.
        let toggles: Vec<Toggle> = vec![
            (toggle("format-text-bold-symbolic", "Bold"), Change::Bold),
            (
                toggle("format-text-italic-symbolic", "Italic"),
                Change::Italic,
            ),
            (
                toggle("format-text-underline-symbolic", "Underline"),
                Change::Underline,
            ),
            (
                toggle("format-text-strikethrough-symbolic", "Strikethrough"),
                Change::Strike,
            ),
            (toggle(CODE_ICON, "Monospace"), Change::Code),
        ];
        let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        group.add_css_class("linked");
        for (button, _) in &toggles {
            group.append(button);
        }

        // Every family Pango can resolve, rather than a list this file curates: a curated one
        // ages, and a document written elsewhere names whatever font its author had. Sorted,
        // and searchable, because "every family installed" is a long list.
        let families = gtk::StringList::new(&[DEFAULT]);
        for name in installed_families(context) {
            families.append(&name);
        }
        let family = gtk::DropDown::builder()
            .model(&families)
            .tooltip_text("Font")
            .enable_search(true)
            .expression(text_expression())
            .build();
        family.set_size_request(150, -1);

        let sizes = gtk::StringList::new(&[]);
        for size in self::sizes(None) {
            sizes.append(&size);
        }
        let size = gtk::DropDown::builder()
            .model(&sizes)
            .tooltip_text("Font size")
            .build();

        let color = Swatch::new("Text colour", true);
        let highlight = Swatch::new("Highlight", false);
        let clear = gtk::Button::from_icon_name("edit-clear-symbolic");
        clear.set_tooltip_text(Some("Clear Formatting"));

        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .margin_start(6)
            .margin_end(6)
            .margin_top(4)
            .margin_bottom(4)
            .build();
        widget.append(&group);
        widget.append(&family);
        widget.append(&size);
        widget.append(&color.button);
        widget.append(&highlight.button);
        widget.append(&clear);

        let bar = Rc::new(Bar {
            widget,
            toggles,
            family,
            families,
            size,
            sizes,
            color,
            highlight,
            updating: Cell::new(false),
            apply: RefCell::new(None),
        });

        for (button, change) in &bar.toggles {
            let change = *change;
            button.connect_toggled(glib::clone!(
                #[strong]
                bar,
                move |button| bar.ask(change(button.is_active()))
            ));
        }
        for (drop_down, change) in [
            (
                &bar.family,
                (Change::Family) as fn(Option<String>) -> Change,
            ),
            (&bar.size, Change::Size),
        ] {
            drop_down.connect_selected_notify(glib::clone!(
                #[strong]
                bar,
                move |drop_down| bar.ask(change(chosen(drop_down)))
            ));
        }
        for (swatch, change) in [
            (&bar.color, (Change::Color) as fn(Option<String>) -> Change),
            (&bar.highlight, Change::Highlight),
        ] {
            swatch.connect(glib::clone!(
                #[strong]
                bar,
                move |value| bar.ask(change(value))
            ));
        }
        clear.connect_clicked(glib::clone!(
            #[strong]
            bar,
            move |_| bar.ask(Change::Clear)
        ));
        bar
    }

    /// Where every control ends up: ask the window to make this change to the selection —
    /// unless [`Bar::show`] is the one moving the control, in which case it is the document
    /// talking to the bar and not the other way round.
    fn ask(&self, change: Change) {
        if self.updating.get() {
            return;
        }
        let apply = self.apply.borrow().clone();
        if let Some(apply) = apply {
            apply(change);
        }
    }

    /// Hand the bar the window it writes through. Called once, from `Ui::wire`.
    pub fn connect(&self, apply: impl Fn(Change) + 'static) {
        *self.apply.borrow_mut() = Some(Rc::new(apply));
    }

    /// Paint the bar from the selection's own formatting.
    ///
    /// `enabled` is whether there is a selection at all: with none, every control is
    /// insensitive rather than lying about a range that is not there. The style is still shown,
    /// because [`grind_text::App::char_style`] answers a bare caret with what the *next*
    /// keystroke would carry, and a toolbar that shows that is telling the truth.
    pub fn show(&self, style: &CharStyle, enabled: bool) {
        self.updating.set(true);
        for (button, change) in &self.toggles {
            button.set_sensitive(enabled);
            let mut probe = style.clone();
            change(true).apply(&mut probe);
            // Pressed when turning this control *on* would change nothing — one rule for all
            // five, so Code lights up for a monospace run exactly as Bold does for a bold one.
            button.set_active(probe == *style);
        }
        for widget in [
            self.family.upcast_ref::<gtk::Widget>(),
            self.size.upcast_ref(),
        ] {
            widget.set_sensitive(enabled);
        }
        self.color.show(style.color.as_deref());
        self.highlight.show(style.background.as_deref());
        self.color.button.set_sensitive(enabled);
        self.highlight.button.set_sensitive(enabled);

        select(&self.family, &self.families, style.font_family.as_deref());
        // The document's own size may not be on the ladder, and it has to be selectable while
        // it is the selection's — so the list is rebuilt around it rather than the value lost.
        let wanted = sizes(style.font_size.as_deref());
        if list(&self.sizes) != wanted {
            self.sizes.splice(
                0,
                self.sizes.n_items(),
                &wanted.iter().map(String::as_str).collect::<Vec<_>>(),
            );
        }
        select(&self.size, &self.sizes, style.font_size.as_deref());
        self.updating.set(false);
    }
}

/// One colour button: the run's colour on its face, [`PALETTE`] as the choices, *Automatic* to
/// take the attribute off again, and a dialog behind *Custom…*.
///
/// The palette lives in the core so that `grind text format --color navy` and this button's
/// navy swatch write the same attribute; it is a default a shell offers, never a limit.
struct Swatch {
    button: gtk::MenuButton,
    /// The colour drawn on the button's face. A `Cell` because the draw function reads it on
    /// every frame.
    shown: Rc<Cell<Option<gdk::RGBA>>>,
    face: gtk::DrawingArea,
    popover: gtk::Popover,
}

impl Swatch {
    fn new(tooltip: &str, text: bool) -> Rc<Self> {
        let shown: Rc<Cell<Option<gdk::RGBA>>> = Rc::new(Cell::new(None));
        let face = area(shown.clone(), 16, 4);

        let head = gtk::Image::from_icon_name(match text {
            true => "format-text-rich-symbolic",
            false => "color-select-symbolic",
        });
        let stack = gtk::Box::new(gtk::Orientation::Vertical, 2);
        stack.set_halign(gtk::Align::Center);
        stack.append(&head);
        stack.append(&face);

        let popover = gtk::Popover::new();
        let button = gtk::MenuButton::builder()
            .child(&stack)
            .tooltip_text(tooltip)
            .popover(&popover)
            .build();
        button.add_css_class("flat");

        Rc::new(Swatch {
            button,
            shown,
            face,
            popover,
        })
    }

    /// The choices are built here rather than in [`Swatch::new`] because every one of them is a
    /// call into the window, and the window does not exist until [`Bar::connect`].
    fn connect(&self, pick: impl Fn(Option<String>) + 'static) {
        self.popover
            .set_child(Some(&choices(&self.popover, Rc::new(pick))));
    }

    fn show(&self, value: Option<&str>) {
        self.shown.set(value.and_then(color));
        self.face.queue_draw();
    }
}

/// The palette grid inside a swatch's popover: every colour, then *Automatic* and *Custom…*.
fn choices(popover: &gtk::Popover, pick: Rc<dyn Fn(Option<String>)>) -> gtk::Grid {
    let grid = gtk::Grid::builder()
        .row_spacing(4)
        .column_spacing(4)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();
    // Six to a row, which is the palette's own order — a user reading clrs.cc and a user
    // reading this menu see the same thing.
    for (index, (name, hex)) in PALETTE.iter().enumerate() {
        let Some(rgba) = color(hex) else { continue };
        let swatch = gtk::Button::builder()
            .child(&area(Rc::new(Cell::new(Some(rgba))), 20, 20))
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
        grid.attach(&swatch, (index % 6) as i32, (index / 6) as i32, 1, 1);
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
    grid.attach(&automatic, 0, 3, 3, 1);

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
                Some(&gdk::RGBA::BLACK),
                gtk::gio::Cancellable::NONE,
                move |chosen| {
                    if let Ok(rgba) = chosen {
                        pick(Some(hex(rgba)));
                    }
                },
            );
        }
    ));
    grid.attach(&custom, 3, 3, 3, 1);
    grid
}

fn toggle(icon: &str, tooltip: &str) -> gtk::ToggleButton {
    gtk::ToggleButton::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .build()
}

/// A swatch of colour, drawn rather than themed — a `GtkButton` with a background is a fight
/// with the stylesheet, and a `DrawingArea` is the colour and nothing else. `None` draws the
/// checkerboard-free empty face, which is what "the attribute is not set" looks like.
fn area(shown: Rc<Cell<Option<gdk::RGBA>>>, width: i32, height: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .content_width(width)
        .content_height(height)
        .build();
    area.set_draw_func(move |area, cr, w, h| {
        let rgba = shown.get().unwrap_or_else(|| {
            let color = area.color();
            gdk::RGBA::new(color.red(), color.green(), color.blue(), 0.35)
        });
        cr.set_source_rgba(
            f64::from(rgba.red()),
            f64::from(rgba.green()),
            f64::from(rgba.blue()),
            f64::from(rgba.alpha()),
        );
        cr.rectangle(0.0, 0.0, f64::from(w), f64::from(h));
        let _ = cr.fill();
    });
    area
}

/// Every font family Pango knows about, sorted — what the drop-down offers.
///
/// Asked of the widget's own context rather than of a fresh one: a bare `pango::Context` has no
/// font map, and the list a document is drawn from has to be the list it is chosen from.
fn installed_families(context: &pango::Context) -> Vec<String> {
    let mut names: Vec<String> = context
        .list_families()
        .iter()
        .map(|family| family.name().to_string())
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup();
    names
}

/// The expression a searchable [`gtk::DropDown`] matches against — the string a
/// [`gtk::StringObject`] holds, which is all this list has.
fn text_expression() -> gtk::Expression {
    gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        gtk::Expression::NONE,
        "string",
    )
    .upcast()
}

/// What a drop-down currently has selected, as a document would store it — [`DEFAULT`] is
/// `None`, which is the attribute being absent.
fn chosen(drop_down: &gtk::DropDown) -> Option<String> {
    let value = drop_down
        .selected_item()
        .and_downcast::<gtk::StringObject>()?
        .string()
        .to_string();
    (value != DEFAULT).then_some(value)
}

/// Select `value` in `list`, falling back to [`DEFAULT`] where the document has none — or where
/// it has one nothing offers, which for a family means a font this machine has not got.
fn select(drop_down: &gtk::DropDown, list: &gtk::StringList, value: Option<&str>) {
    let wanted = value.unwrap_or(DEFAULT);
    let index = list
        .iter::<glib::Object>()
        .flatten()
        .position(|item| {
            item.downcast::<gtk::StringObject>()
                .is_ok_and(|s| s.string() == wanted)
        })
        .unwrap_or(0);
    drop_down.set_selected(index as u32);
}

fn list(model: &gtk::StringList) -> Vec<String> {
    model
        .iter::<glib::Object>()
        .flatten()
        .filter_map(|item| {
            item.downcast::<gtk::StringObject>()
                .ok()
                .map(|s| s.string().to_string())
        })
        .collect()
}

/// An ODF colour as GDK reads it — `#rrggbb`, and the names GDK already knows. `transparent`
/// is a value rather than a colour, and answers `None` so the face draws as unset.
fn color(value: &str) -> Option<gdk::RGBA> {
    (!value.eq_ignore_ascii_case("transparent"))
        .then(|| gdk::RGBA::parse(value).ok())
        .flatten()
}

/// A colour as a document stores it. The dialog answers in floats and ODF wants `#rrggbb`.
fn hex(rgba: gdk::RGBA) -> String {
    let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(rgba.red()),
        channel(rgba.green()),
        channel(rgba.blue())
    )
}

/// A palette name as a menu shows it. The names are ASCII and lower-case by construction
/// (`PALETTE`), so this is the whole of the transformation.
fn capitalised(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bar's whole vocabulary, as a specification: what each control writes, and what it
    /// leaves alone. No display needed, which is the point of `Change` being a value.
    #[test]
    fn every_control_touches_one_property_and_clear_touches_all_of_them() {
        let mut style = CharStyle::default();
        Change::Bold(true).apply(&mut style);
        Change::Color(Some("#ff4136".into())).apply(&mut style);
        assert!(style.is_bold());
        assert_eq!(style.color.as_deref(), Some("#ff4136"));

        // Italic leaves the bold and the colour exactly as they were.
        Change::Italic(true).apply(&mut style);
        assert!(style.is_bold() && style.is_italic());
        assert_eq!(style.color.as_deref(), Some("#ff4136"));

        // Code is the monospace family, which is what the notation writes.
        Change::Code(true).apply(&mut style);
        assert_eq!(
            style.font_family.as_deref(),
            Some(grind_text::markdown::MONOSPACE)
        );
        Change::Code(false).apply(&mut style);
        assert_eq!(style.font_family, None, "off means absent, not \"sans\"");

        Change::Clear.apply(&mut style);
        assert!(style.is_plain(), "one shot, everything off");
    }

    /// A size the document chose that is not on the ladder still has to be selectable, or the
    /// drop-down would silently rewrite it the moment somebody clicked anything else.
    #[test]
    fn the_size_list_makes_room_for_a_size_the_document_chose() {
        let usual = sizes(None);
        assert_eq!(usual[0], DEFAULT);
        assert!(usual.contains(&"12pt".to_owned()));
        assert!(!usual.contains(&"13pt".to_owned()));

        let odd = sizes(Some("13pt"));
        assert_eq!(odd[0], DEFAULT);
        assert_eq!(odd[1], "13pt", "the document's own, right where it is used");
        // And one already on the ladder is not doubled.
        assert_eq!(sizes(Some("12pt")), usual);
    }

    #[test]
    fn a_colour_round_trips_through_the_dialogs_own_spelling() {
        assert_eq!(hex(gdk::RGBA::new(1.0, 0.0, 0.0, 1.0)), "#ff0000");
        assert!(color("transparent").is_none(), "a value, not a colour");
        assert!(color("#001f3f").is_some());
    }

    #[test]
    fn a_palette_name_is_capitalised_for_a_menu() {
        assert_eq!(capitalised("navy"), "Navy");
        assert_eq!(capitalised(""), "");
    }
}
