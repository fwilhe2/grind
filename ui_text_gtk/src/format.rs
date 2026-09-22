// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The formatting bar — every property of a *run* this window can read and write.
//!
//! `ui_sheet_gtk/src/formatting.rs`'s counterpart, one document type over, and it earns its
//! place by the same admission test `doc/sheet-shell.md` sets for that shell's format bar: a
//! control belongs here when it **reads and writes a property of the selection**, and nowhere
//! else. `CharStyle`'s eight fields and the caret's block *kind* are therefore the whole bound
//! of this file — there is no room here for a verb, and a verb that wants a home goes in the
//! menu.
//!
//! The kind is the one control that is not a `CharStyle` field, and it is first on the bar
//! because it is the one a writer reaches for most: *make this a heading*. It lived in a
//! submenu of the primary menu, three clicks away and invisible, while the bar beside it showed
//! the font size — and a heading is what a document's outline, its addresses (`§2.1`) and its
//! look are all built from.
//!
//! **With nothing selected the bar still works**: a control pressed at a bare caret sets the
//! style the *next character typed* will carry, as every word processor does, rather than
//! greying out and leaving Bold-then-type silently unbold. See `Ui::apply_char_style`.
//!
//! **Every control reads the document and writes the document, and keeps nothing.** The toggles
//! show what [`grind_text::App::char_style`] says the selection agrees about and write through
//! `set_char_style`; the two swatches and the two drop-downs do exactly the same with the four
//! properties that carry a value rather than a boolean. No widget here holds an opinion about
//! what is bold — there is one such fact and it is in the document.
//!
//! What is testable without a display lives in [`grind_text::format::Change`], the whole
//! vocabulary of the bar as a pure function over a `CharStyle`, and in
//! [`grind_text::format::sizes`] — hoisted into `grind-text` itself the day `grind-win32`
//! wanted the same answers, the way `sheet/src/formula/assist.rs` was. The widgets around them
//! need GTK and are exercised by `view.rs`'s one widget harness.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use libadwaita::gtk;
use libadwaita::prelude::*;

use grind_core::style::PALETTE;
use grind_text::CharStyle;
pub use grind_text::format::{Change, DEFAULT, sizes};
use gtk::{gdk, glib, pango};

/// The Monospace toggle's icon.
///
/// Adwaita has no `code-symbolic`, and an icon name the theme has never heard of draws as the
/// missing-image placeholder rather than failing — which is a bug you only find by looking at a
/// screenshot, and this one was found exactly that way. The terminal icon is in Adwaita's own
/// set and reads as "fixed width" to anybody who would reach for this button.
const CODE_ICON: &str = "utilities-terminal-symbolic";

/// The block kinds the bar's first control offers, each with the `win.` action that applies it
/// — the same actions the keys (Ctrl+0…3, Ctrl+L) reach, so the picker adds no second way of
/// making a heading, only a visible one.
pub const BLOCK_STYLES: [(&str, &str); 10] = [
    ("Paragraph", "win.paragraph"),
    ("Title", "win.title"),
    ("Subtitle", "win.subtitle"),
    ("Heading 1", "win.heading-1"),
    ("Heading 2", "win.heading-2"),
    ("Heading 3", "win.heading-3"),
    ("Heading 4", "win.heading-4"),
    ("Heading 5", "win.heading-5"),
    ("Heading 6", "win.heading-6"),
    ("List Item", "win.list-item"),
];

/// Which of [`BLOCK_STYLES`] a block is: its kind, and for a paragraph the two named styles this
/// window draws a face of its own for. A paragraph under any other name is a Paragraph — the
/// name is kept (`Ui::set_kind`) and simply is not one this picker offers.
pub fn block_style(kind: &grind_text::BlockKind, style: Option<&str>) -> u32 {
    use grind_text::BlockKind;
    match kind {
        BlockKind::Paragraph => match style {
            Some("Title") => 1,
            Some("Subtitle") => 2,
            _ => 0,
        },
        BlockKind::Heading { level } => 2 + (*level).clamp(1, 6),
        BlockKind::ListItem { .. } => 9,
    }
}

/// One toggle and the [`Change`] it asks for when pressed.
type Toggle = (gtk::ToggleButton, fn(bool) -> Change);

/// Where a control's [`Change`] goes — the window, once [`Bar::connect`] has named one.
type Apply = Rc<dyn Fn(Change)>;

/// The formatting bar's widgets, and the one callback they all reach the document through.
pub struct Bar {
    pub widget: gtk::Box,
    /// The caret's block kind — [`BLOCK_STYLES`].
    block: gtk::DropDown,
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
        let labels: Vec<&str> = BLOCK_STYLES.iter().map(|(label, _)| *label).collect();
        let block = gtk::DropDown::from_strings(&labels);
        block.set_tooltip_text(Some("Paragraph Style"));
        block.set_size_request(140, -1);

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

        // The `toolbar` class is what makes the buttons in it flat, the way the spreadsheet's
        // format bar has always drawn — without it Clear Formatting was the one raised button
        // on the bar and read as its default action.
        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .build();
        widget.add_css_class("toolbar");
        widget.append(&block);
        widget.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        widget.append(&group);
        widget.append(&family);
        widget.append(&size);
        widget.append(&color.button);
        widget.append(&highlight.button);
        widget.append(&clear);

        let bar = Rc::new(Bar {
            widget,
            block,
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
        // The kind goes through the window's own action rather than through `apply`, since it
        // is a block's property and not a run's: the same action Ctrl+1 activates.
        bar.block.connect_selected_notify(glib::clone!(
            #[strong]
            bar,
            move |drop_down| {
                if bar.updating.get() {
                    return;
                }
                if let Some((_, action)) = BLOCK_STYLES.get(drop_down.selected() as usize) {
                    let _ = drop_down.activate_action(action, None);
                }
            }
        ));
        bar
    }

    /// Show the caret's block kind — [`block_style`]'s index.
    pub fn show_block(&self, index: u32) {
        self.updating.set(true);
        self.block.set_selected(index);
        self.updating.set(false);
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

    /// Paint the bar from the selection's own formatting — or, with nothing selected, from what
    /// the next character typed will carry, which is what a control pressed there would change.
    pub fn show(&self, style: &CharStyle) {
        self.updating.set(true);
        for (button, change) in &self.toggles {
            let mut probe = style.clone();
            change(true).apply(&mut probe);
            // Pressed when turning this control *on* would change nothing — one rule for all
            // five, so Code lights up for a monospace run exactly as Bold does for a bold one.
            button.set_active(probe == *style);
        }
        self.color.show(style.color.as_deref());
        self.highlight.show(style.background.as_deref());

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

    // `Change`'s own vocabulary and `sizes`' ladder are specified and tested in
    // `grind_text::format` now, which is where they are defined.

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
