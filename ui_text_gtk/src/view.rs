// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The document view: the pages as they will print, on a desk, with a caret, and owning
//! neither. The pages are [`grind_print`]'s, set by a [`grind_print::session::Session`] and
//! drawn from the very raster Print Preview shows; what is drawn over them — caret, selection,
//! underlines — and every question about a point or a line is [`grind_print::place`]'s, over the
//! line boxes the pages were set with (`doc/page-model.md` PM5).
//!
//! **Not `GtkTextView`**, whose `GtkTextBuffer` would be a second copy of the document with its
//! own undo stack and no notion of a `text:h`.
//!
//! Every edit is the core's: typing [`App::insert_text`], Enter [`App::split_block`],
//! Backspace at a block's front [`App::erase`] across the boundary.
//!
//! Typing goes through `GtkIMMulticontext`, so dead keys, compose sequences and input methods
//! work; [`crate::keymap`] never sees a printable character.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use libadwaita::gtk;
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::ObjectSubclassIsExt;

use grind_text::{App, Caret};
use gtk::glib;

glib::wrapper! {
    pub struct Doc(ObjectSubclass<imp::Doc>)
        @extends gtk::Widget,
        @implements gtk::Scrollable, gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Doc {
    pub fn new(app: Arc<App>) -> Self {
        let doc: Self = glib::Object::builder().build();
        doc.imp().app.replace(Some(app));
        doc
    }

    /// Draw underlines under the words `speller` does not know — the same speller the window
    /// attached to the `App` — or with `None` none at all.
    pub fn set_speller(&self, speller: Option<Arc<grind_spell::Speller>>) {
        self.imp().speller.replace(speller);
        self.queue_draw();
    }

    /// Leave these grammar rules undrawn — the person's ignored ones (`doc/grammar.md`
    /// decision 5).
    pub fn set_grammar_ignored(&self, rules: Vec<String>) {
        self.imp().grammar_ignored.replace(rules);
        self.queue_draw();
    }

    /// The document changed: forget everything measured from it and repaint. Called from the
    /// observer, so it covers this widget's own edits too (rule 3).
    pub fn invalidate(&self) {
        self.imp().session.changed();
        self.imp().clamp_caret();
        // The observer runs *after* the edit's own caret move, which followed the caret
        // through the layout as it was before the edit — so Enter on the last line left the
        // new one below the fold. Follow it again through the layout as it is now.
        self.imp().scroll_into_view();
        // The document's height changed, so the scrollbar has to be sized again — which is
        // what allocation does, and it has the width and height to do it with.
        self.queue_allocate();
        self.queue_draw();
    }

    /// A different document: back to the top, with no goal column carried over.
    pub fn reset(&self) {
        let imp = self.imp();
        imp.caret.set(Caret {
            block: 0,
            offset: 0,
        });
        imp.anchor.set(None);
        imp.goal_x.set(None);
        if let Some(adjustment) = imp.vadjustment.borrow().as_ref() {
            adjustment.set_value(0.0);
        }
        self.invalidate();
    }

    pub fn caret(&self) -> Caret {
        self.imp().caret.get()
    }

    /// Whether the bookmark anchors are drawn — `doc/view-modes.md` §3.6.
    pub fn names(&self) -> bool {
        self.imp().names.get()
    }

    /// Draw them, or stop. A reading of the document rather than a change to it: the file is
    /// byte-identical either way, which is why this needs no confirmation and leaves no undo
    /// entry behind it.
    pub fn set_names(&self, on: bool) {
        self.imp().names.set(on);
        self.queue_draw();
    }

    /// The selected range, normalised to document order — `None` when the anchor and the
    /// caret coincide, which is what makes an empty selection and no selection the same case
    /// everywhere else in this file.
    pub fn selection(&self) -> Option<(Caret, Caret)> {
        self.imp().selection()
    }

    /// Select `from..to` with the caret at `to` — a found word, say. Scrolled into view like
    /// any other caret move.
    pub fn select(&self, from: Caret, to: Caret) {
        let imp = self.imp();
        imp.anchor.set(Some(from));
        imp.move_caret(to, true);
    }

    /// Select the whole document, first block to last.
    pub fn select_all(&self) {
        let imp = self.imp();
        let Some(app) = imp.app() else { return };
        let count = app.block_count();
        if count == 0 {
            return;
        }
        let last = count - 1;
        let end = app.input_text(last).map_or(0, |text| text.chars().count());
        imp.anchor.set(Some(Caret {
            block: 0,
            offset: 0,
        }));
        imp.move_caret(
            Caret {
                block: last,
                offset: end,
            },
            true,
        );
    }

    /// The style the next character typed will carry, when one is pending — a markdown span
    /// that just closed, or a formatting control pressed with nothing selected.
    pub fn pending(&self) -> Option<grind_text::CharStyle> {
        self.imp().resume.borrow().clone()
    }

    /// Hold `style` for the next character typed at the caret. Forgotten as soon as the caret
    /// moves anywhere but forward by typing ([`imp::Doc::move_caret`]).
    pub fn set_pending(&self, style: grind_text::CharStyle) {
        self.imp().resume.replace(Some(style));
        for hook in self.imp().on_moved.borrow().iter() {
            hook(self.imp().caret.get());
        }
    }

    /// Told whenever the caret moves — the status bar's readout.
    pub fn connect_moved(&self, f: impl Fn(Caret) + 'static) {
        self.imp().on_moved.borrow_mut().push(Box::new(f));
    }

    /// Told when an edit was refused, with the core's own message. A toast, never a dialog:
    /// nothing here is a question.
    pub fn connect_notice(&self, f: impl Fn(String) + 'static) {
        self.imp().on_notice.borrow_mut().push(Box::new(f));
    }

    /// Told a link's target when somebody follows it — Ctrl+click. Where it goes is the
    /// window's business: a URL opens outside, a `#name` is a jump inside.
    pub fn connect_link(&self, f: impl Fn(String) + 'static) {
        self.imp().on_link.borrow_mut().push(Box::new(f));
    }

    /// The caret's rectangle in this widget's coordinates — what a popover about the caret
    /// points at. `None` before the first allocation.
    pub fn caret_rect(&self) -> Option<gtk::gdk::Rectangle> {
        self.imp().caret_rect()
    }

    /// The caret's page and how many there are, counted from one for a reader.
    pub fn page_of_caret(&self) -> Option<(usize, usize)> {
        let shown = self.imp().shown()?;
        let spot = grind_print::place::locate(&shown.set, self.caret())?;
        Some((spot.page + 1, shown.set.pages.len()))
    }

    /// How large the pages are drawn: 1.0 is their printed size.
    pub fn zoom(&self) -> f64 {
        self.imp().zoom.get()
    }

    /// Draw the pages at `zoom`, keeping what is under `anchor` (a point in the widget, its
    /// middle when `None`) where it is. Never written to the document.
    pub fn set_zoom(&self, zoom: f64, anchor: Option<(f64, f64)>) {
        let imp = self.imp();
        let levels = crate::strip::ZOOMS;
        let zoom = zoom.clamp(levels[0], levels[levels.len() - 1]);
        let (width, height) = (self.width(), self.height());
        let (ax, ay) = anchor.unwrap_or((f64::from(width) / 2.0, f64::from(height) / 2.0));
        let (left, top) = imp.scrolled();
        let held = imp.shown().and_then(|shown| {
            let page = shown.strip.page_at(ay + top)?;
            Some((page, shown.strip.on_page(page, ax + left, ay + top)?))
        });
        imp.zoom.set(zoom);
        imp.size_scrollbars(width, height);
        if let (Some((page, (x, y))), Some(shown)) = (held, imp.shown())
            && let Some((x, y)) = shown.strip.in_strip(page, x, y)
        {
            if let Some(adjustment) = imp.hadjustment.borrow().as_ref() {
                adjustment.set_value(x - ax);
            }
            if let Some(adjustment) = imp.vadjustment.borrow().as_ref() {
                adjustment.set_value(y - ay);
            }
        }
        self.queue_draw();
        for hook in imp.on_zoomed.borrow().iter() {
            hook(zoom);
        }
    }

    /// One zoom level in or out, about the caret when it is in view.
    pub fn zoom_step(&self, inward: bool) {
        let anchor = self
            .imp()
            .point_of(self.caret())
            .filter(|&(x, y, _)| {
                (0.0..f64::from(self.width())).contains(&x)
                    && (0.0..f64::from(self.height())).contains(&y)
            })
            .map(|(x, y, _)| (x, y));
        self.set_zoom(crate::strip::zoom_step(self.zoom(), inward), anchor);
    }

    /// The zoom at which the widest page fills the window's width.
    pub fn fit_width(&self) {
        let Some(shown) = self.imp().shown() else {
            return;
        };
        let widest = shown
            .set
            .pages
            .iter()
            .map(|page| f64::from(page.width))
            .fold(0.0, f64::max);
        let scale = f64::from(crate::print::pixels_per_point(1.0, 1.0));
        let zoom = crate::strip::fit_width(widest, f64::from(self.width()), scale);
        self.set_zoom(zoom, None);
    }

    /// The zoom at which the caret's page, or the first, fits the window whole.
    pub fn fit_page(&self) {
        let Some(shown) = self.imp().shown() else {
            return;
        };
        let at = self.page_of_caret().map_or(0, |(page, _)| page - 1);
        let Some(page) = shown.set.pages.get(at) else {
            return;
        };
        let scale = f64::from(crate::print::pixels_per_point(1.0, 1.0));
        let zoom = crate::strip::fit_page(
            (f64::from(page.width), f64::from(page.height)),
            (f64::from(self.width()), f64::from(self.height())),
            scale,
        );
        self.set_zoom(zoom, None);
        self.show_page(at);
    }

    /// Scroll page `page`, from 0, to the top of the view — now, or once the view has a size.
    pub fn show_page(&self, page: usize) {
        let imp = self.imp();
        if self.width() == 0 {
            imp.pending_page.set(Some(page));
            return;
        }
        let (Some(shown), Some(adjustment)) = (imp.shown(), imp.vadjustment.borrow().clone())
        else {
            return;
        };
        if let Some(sheet) = shown.strip.sheet(page) {
            adjustment.set_value(sheet.y - crate::strip::PAD);
        }
    }

    /// Told the zoom whenever it changes — the status bar's readout.
    pub fn connect_zoomed(&self, f: impl Fn(f64) + 'static) {
        self.imp().on_zoomed.borrow_mut().push(Box::new(f));
    }

    /// Set the caret from an address the user typed — `p12`, `#intro`, `§2.1.3`. A jump
    /// replaces the caret rather than extending anything, so any selection goes with it.
    pub fn go_to(&self, caret: Caret) {
        self.imp().anchor.set(None);
        self.imp().move_caret(caret, true);
    }

    /// One step deeper into a list, or one step out of it. `false` when the caret's block is
    /// not a list item and this was not the gesture that makes one — see [`imp::Doc::indent`].
    pub fn indent(&self, by: i32) -> bool {
        self.imp().indent(by)
    }

    /// Put the selection on the system clipboard; `false` when nothing is selected. Plain text: a
    /// run's `CharStyle` has no spelling in `text/plain`.
    pub fn copy(&self) -> bool {
        let Some(text) = self.imp().selected_text() else {
            return false;
        };
        self.clipboard().set_text(&text);
        true
    }

    /// Copy, then erase what was copied.
    pub fn cut(&self) -> bool {
        if !self.copy() {
            return false;
        }
        self.imp().erase_back();
        true
    }

    /// Read the clipboard and put its text in at the caret, replacing the selection —
    /// asynchronously, since the bytes may come from another process.
    pub fn paste(&self) {
        self.clipboard().read_text_async(
            gtk::gio::Cancellable::NONE,
            glib::clone!(
                #[weak(rename_to = doc)]
                self,
                move |result| {
                    if let Ok(Some(text)) = result {
                        doc.imp().paste_text(&text);
                    }
                }
            ),
        );
    }
}

/// How many suggestions the right-click menu offers for a misspelt word. Hunspell ranks them,
/// and past the first few they are rarely the word that was meant.
pub const SUGGESTIONS: usize = 6;

/// The spelling section leading the right-click menu over a misspelt word: the suggestions, each
/// `spell.correct` with the word as its target, then Ignore All and Add to Dictionary.
pub fn spelling_menu_model(suggestions: &[String]) -> gtk::gio::Menu {
    let model = gtk::gio::Menu::new();
    let offered = gtk::gio::Menu::new();
    for word in suggestions.iter().take(SUGGESTIONS) {
        let item = gtk::gio::MenuItem::new(Some(word), None);
        item.set_action_and_target_value(Some("spell.correct"), Some(&word.to_variant()));
        offered.append_item(&item);
    }
    if suggestions.is_empty() {
        let none = gtk::gio::MenuItem::new(Some("No Suggestions"), Some("spell.none"));
        offered.append_item(&none);
    }
    model.append_section(None, &offered);
    let keep = gtk::gio::Menu::new();
    keep.append(Some("Ignore All"), Some("spell.ignore"));
    keep.append(Some("Add to Dictionary"), Some("spell.add"));
    model.append_section(None, &keep);
    model
}

/// The grammar section leading the right-click menu over a finding: what is wrong (not an action),
/// each replacement as `grammar.correct` with it as the target, then Ignore Rule.
pub fn grammar_menu_model(finding: &grind_text::grammar::Finding) -> gtk::gio::Menu {
    let model = gtk::gio::Menu::new();
    let what = gtk::gio::Menu::new();
    what.append(Some(&finding.message), Some("grammar.none"));
    model.append_section(None, &what);
    let offered = gtk::gio::Menu::new();
    for with in finding.replacements.iter().take(SUGGESTIONS) {
        let label = match with.trim().is_empty() {
            true if with.is_empty() => "Remove".to_owned(),
            true => "One Space".to_owned(),
            false => with.clone(),
        };
        let item = gtk::gio::MenuItem::new(Some(&label), None);
        item.set_action_and_target_value(Some("grammar.correct"), Some(&with.to_variant()));
        offered.append_item(&item);
    }
    if offered.n_items() > 0 {
        model.append_section(None, &offered);
    }
    let keep = gtk::gio::Menu::new();
    keep.append(Some("Ignore Rule"), Some("grammar.ignore"));
    model.append_section(None, &keep);
    model
}

/// A picture's bytes decoded for painting — what the page draws, and what the Alt Text dialog
/// shows.
pub fn texture(image: &grind_text::ImageView) -> Option<gtk::gdk::Texture> {
    gtk::gdk::Texture::from_bytes(&glib::Bytes::from(&image.data)).ok()
}

/// The right-click menu on the page: the clipboard, and nothing about the document as a whole,
/// which is the primary menu's. A free function so a test can walk it without a display.
pub fn context_menu_model() -> gtk::gio::Menu {
    let model = gtk::gio::Menu::new();
    let clipboard = gtk::gio::Menu::new();
    clipboard.append(Some("Cut"), Some("win.cut"));
    clipboard.append(Some("Copy"), Some("win.copy"));
    clipboard.append(Some("Paste"), Some("win.paste"));
    model.append_section(None, &clipboard);
    let selection = gtk::gio::Menu::new();
    selection.append(Some("Select All"), Some("win.select-all"));
    model.append_section(None, &selection);
    let paragraph = gtk::gio::Menu::new();
    let link = gtk::gio::Menu::new();
    link.append(Some("Link…"), Some("win.link"));
    // Greyed unless the caret is beside a picture, which a right-click on one makes it.
    link.append(Some("Alt Text…"), Some("win.alt-text"));
    model.append_section(None, &link);
    paragraph.append(Some("Move Paragraph Up"), Some("win.paragraph-up"));
    paragraph.append(Some("Move Paragraph Down"), Some("win.paragraph-down"));
    paragraph.append(Some("Delete Paragraph"), Some("win.paragraph-delete"));
    paragraph.append(Some("Bookmark Here…"), Some("win.bookmark"));
    paragraph.append(Some("Paragraph Style Name…"), Some("win.style-name"));
    model.append_section(None, &paragraph);
    model
}

mod imp {
    use super::*;

    use std::collections::HashMap;
    use std::ops::Range;

    use grind_print::place;
    use grind_text::{BlockKind, loc};
    use gtk::subclass::prelude::*;
    use gtk::{gdk, graphene, gsk};

    use crate::keymap::{self, Action, Key, Mods, Motion};
    use crate::strip::{self, Strip};
    use crate::theme::Palette;

    type NoticeHook = Box<dyn Fn(String)>;
    type MovedHook = Box<dyn Fn(Caret)>;
    type ZoomedHook = Box<dyn Fn(f64)>;

    /// How thick the caret is, in pixels.
    const CARET: f64 = 1.5;

    pub struct Doc {
        pub app: RefCell<Option<Arc<App>>>,
        pub caret: Cell<Caret>,
        /// The other end of the selection, when there is one. `None` means "no selection",
        /// not "a selection at the caret" — see [`Doc::selection`], the one place that turns
        /// this and the caret into a range.
        pub anchor: Cell<Option<Caret>>,
        /// The page x the caret is trying to keep while moving by lines ([`place::step`]).
        /// Cleared by any horizontal move, which is what makes walking down through a short
        /// line and out the other side come back where it started.
        pub goal_x: Cell<Option<f32>>,
        /// What `App::type_markdown` said the next character must be set in, so a notation
        /// ends where its closing marker does. Handed straight back and never read here.
        pub resume: RefCell<Option<grind_text::CharStyle>>,
        /// The document's pages, kept set between paints and told of every change.
        pub session: grind_print::session::Session,
        /// The pages and where each sits, at the zoom and width they were last asked for.
        pub shown: RefCell<Option<Rc<Shown>>>,
        /// Each page's pixels, kept until the page or the scale changes.
        pub tiles: RefCell<HashMap<usize, Tile>>,
        /// 1.0 is a page at its printed size on the screen.
        pub zoom: Cell<f64>,
        pub palette: Cell<Option<Palette>>,
        /// Whether bookmark anchors are drawn (`doc/view-modes.md` §3.6) — a reading of the
        /// document, so turning it off puts the page back with nothing to save or undo.
        pub names: Cell<bool>,
        pub hadjustment: RefCell<Option<gtk::Adjustment>>,
        pub vadjustment: RefCell<Option<gtk::Adjustment>>,
        pub hscroll_policy: Cell<gtk::ScrollablePolicy>,
        pub vscroll_policy: Cell<gtk::ScrollablePolicy>,
        pub im: gtk::IMMulticontext,
        pub on_notice: RefCell<Vec<NoticeHook>>,
        /// Told a link's target when it is followed — Ctrl+click on one.
        pub on_link: RefCell<Vec<NoticeHook>>,
        pub on_moved: RefCell<Vec<MovedHook>>,
        pub on_zoomed: RefCell<Vec<ZoomedHook>>,
        /// The zoom a pinch started from, while one is under way.
        pub pinched: Cell<Option<f64>>,
        /// A page asked for before the view had a size to scroll it into.
        pub pending_page: Cell<Option<usize>>,
        /// Whether the caret is in the shown half of a blink, and the timer turning it.
        pub lit: Cell<bool>,
        pub blinking: RefCell<Option<glib::SourceId>>,
        /// The last press — its time, where, and how many in a row — so a second press on the same
        /// spot is a double-click. Counted here rather than by a second gesture, since the drag
        /// planting the caret and a click gesture would both answer one press in an order GTK does
        /// not promise.
        pub presses: Cell<Option<(u32, f64, f64, u8)>>,
        /// The right-click menu, built once, parented on this widget and unparented in `dispose`.
        pub menu: std::cell::OnceCell<gtk::PopoverMenu>,
        /// The dictionary the window attached to the `App`, kept here too so Ignore All and Add
        /// to Dictionary can teach it a word (`doc/spelling.md`). `None` draws no underlines.
        pub speller: RefCell<Option<Arc<grind_spell::Speller>>>,
        /// The misspelt word the right-click menu was opened on — what `spell.correct`,
        /// `spell.ignore` and `spell.add` act on.
        pub spot: RefCell<Option<grind_text::Misspelling>>,
        /// The grammar finding the right-click menu was opened on — what `grammar.correct` and
        /// `grammar.ignore` act on.
        pub grammar_spot: RefCell<Option<grind_text::grammar::Flag>>,
        /// The grammar rules the person ignores, which are not drawn.
        pub grammar_ignored: RefCell<Vec<String>>,
        /// What the input method is composing and has not committed, with its own attributes.
        pub preedit: RefCell<Option<(String, gtk::pango::AttrList)>>,
    }

    // Spelled out rather than derived: neither `Caret` nor `ScrollablePolicy` has a
    // `Default`, and the caret's is a decision anyway — a new view starts at the top of the
    // document, which is the one position every document has.
    impl Default for Doc {
        fn default() -> Self {
            Doc {
                app: RefCell::new(None),
                caret: Cell::new(Caret {
                    block: 0,
                    offset: 0,
                }),
                anchor: Cell::new(None),
                goal_x: Cell::new(None),
                resume: RefCell::new(None),
                session: grind_print::session::Session::new(false),
                shown: RefCell::new(None),
                tiles: RefCell::new(HashMap::new()),
                zoom: Cell::new(1.0),
                palette: Cell::new(None),
                names: Cell::new(false),
                hadjustment: RefCell::new(None),
                vadjustment: RefCell::new(None),
                hscroll_policy: Cell::new(gtk::ScrollablePolicy::Minimum),
                vscroll_policy: Cell::new(gtk::ScrollablePolicy::Minimum),
                im: gtk::IMMulticontext::new(),
                on_notice: RefCell::new(Vec::new()),
                on_link: RefCell::new(Vec::new()),
                on_moved: RefCell::new(Vec::new()),
                on_zoomed: RefCell::new(Vec::new()),
                pinched: Cell::new(None),
                pending_page: Cell::new(None),
                lit: Cell::new(true),
                blinking: RefCell::new(None),
                presses: Cell::new(None),
                menu: std::cell::OnceCell::new(),
                speller: RefCell::new(None),
                spot: RefCell::new(None),
                grammar_spot: RefCell::new(None),
                grammar_ignored: RefCell::new(Vec::new()),
                preedit: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Doc {
        const NAME: &'static str = "GrindTextDoc";
        type Type = super::Doc;
        type ParentType = gtk::Widget;
        type Interfaces = (gtk::Scrollable,);

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("grinddoc");
            // `TextBox` is the role for an editable region of text, which is what this is
            // even though it draws itself.
            klass.set_accessible_role(gtk::AccessibleRole::TextBox);
        }
    }

    impl ObjectImpl for Doc {
        // The four properties `GtkScrollable` requires, overridden by hand: the `Properties`
        // derive's spelling has churned between gtk4-rs releases.
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: std::sync::OnceLock<Vec<glib::ParamSpec>> =
                std::sync::OnceLock::new();
            PROPERTIES.get_or_init(|| {
                vec![
                    glib::ParamSpecOverride::for_interface::<gtk::Scrollable>("hadjustment"),
                    glib::ParamSpecOverride::for_interface::<gtk::Scrollable>("vadjustment"),
                    glib::ParamSpecOverride::for_interface::<gtk::Scrollable>("hscroll-policy"),
                    glib::ParamSpecOverride::for_interface::<gtk::Scrollable>("vscroll-policy"),
                ]
            })
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                name @ ("hadjustment" | "vadjustment") => {
                    let adjustment: Option<gtk::Adjustment> = value.get().ok();
                    // A scroll changes nothing in the document, so nothing else would ask
                    // for the repaint that draws the page it moved to.
                    if let Some(adjustment) = &adjustment {
                        adjustment.connect_value_changed(glib::clone!(
                            #[weak(rename_to = doc)]
                            self.obj(),
                            move |_| doc.queue_draw()
                        ));
                    }
                    match name {
                        "hadjustment" => self.hadjustment.replace(adjustment),
                        _ => self.vadjustment.replace(adjustment),
                    };
                }
                "hscroll-policy" => self.hscroll_policy.set(value.get().unwrap()),
                "vscroll-policy" => self.vscroll_policy.set(value.get().unwrap()),
                other => unimplemented!("property {other}"),
            }
            self.obj().queue_allocate();
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "hadjustment" => self.hadjustment.borrow().to_value(),
                "vadjustment" => self.vadjustment.borrow().to_value(),
                "hscroll-policy" => self.hscroll_policy.get().to_value(),
                "vscroll-policy" => self.vscroll_policy.get().to_value(),
                other => unimplemented!("property {other}"),
            }
        }

        /// The one child — the right-click menu — has to be unparented, or GTK complains at
        /// teardown.
        fn dispose(&self) {
            if let Some(menu) = self.menu.get() {
                menu.unparent();
            }
            self.stop_blinking();
        }

        fn constructed(&self) {
            self.parent_constructed();
            let widget = self.obj();
            widget.set_focusable(true);

            self.im.set_client_widget(Some(&*widget));
            self.im.connect_commit(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |_, text| doc.imp().type_text(text)
            ));
            self.im.connect_preedit_changed(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |im| {
                    let (text, attrs, _) = im.preedit_string();
                    attrs.insert(gtk::pango::AttrInt::new_underline(
                        gtk::pango::Underline::Single,
                    ));
                    let composing = (!text.is_empty()).then(|| (text.to_string(), attrs));
                    doc.imp().preedit.replace(composing);
                    doc.queue_draw();
                }
            ));
            self.im.connect_preedit_end(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |_| {
                    doc.imp().preedit.replace(None);
                    doc.queue_draw();
                }
            ));

            let keys = gtk::EventControllerKey::new();
            // Everything this shell does not claim travels on to the input method, which is
            // what turns a keystroke into the text that was actually meant.
            keys.set_im_context(Some(&self.im));
            keys.connect_key_pressed(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_, keyval, _, state| doc.imp().key_pressed(keyval, state)
            ));
            widget.add_controller(keys);

            // The input method has to be told about focus, or a compose sequence started in
            // another window finishes in this one.
            let focus = gtk::EventControllerFocus::new();
            focus.connect_enter(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |_| {
                    doc.imp().im.focus_in();
                    doc.imp().blink();
                }
            ));
            focus.connect_leave(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |_| {
                    doc.imp().im.focus_out();
                    doc.imp().stop_blinking();
                    doc.queue_draw();
                }
            ));
            widget.add_controller(focus);

            // A drag rather than a click: dragging with the button down is how a mouse
            // selects, and a plain click is just a drag whose `drag-update` never fires —
            // one gesture serves both instead of two that would have to agree.
            let drag = gtk::GestureDrag::new();
            drag.connect_drag_begin(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |gesture, x, y| {
                    doc.grab_focus();
                    let shift = gesture
                        .current_event_state()
                        .contains(gtk::gdk::ModifierType::SHIFT_MASK);
                    let count = doc.imp().count_press(gesture.current_event_time(), x, y);
                    let ctrl = gesture
                        .current_event_state()
                        .contains(gtk::gdk::ModifierType::CONTROL_MASK);
                    // Ctrl+click on a link follows it rather than putting the caret in it — a
                    // plain click has to stay a click, or a linked word could not be edited.
                    if count == 1 && ctrl && !shift && doc.imp().follow(x, y) {
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                        return;
                    }
                    match count {
                        1 => doc.imp().click(x, y, shift),
                        // A double-click is the word, a triple-click the paragraph — what every
                        // text field on the desktop does, and so what a hand already expects.
                        2 => doc.imp().select_around(x, y, false),
                        _ => doc.imp().select_around(x, y, true),
                    }
                }
            ));
            drag.connect_drag_update(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |gesture, offset_x, offset_y| {
                    // A word or a paragraph just selected by a multi-click stays selected: the
                    // pointer's jitter between the presses is not a drag.
                    if doc.imp().presses.get().is_some_and(|(.., count)| count > 1) {
                        return;
                    }
                    if let Some((start_x, start_y)) = gesture.start_point() {
                        doc.imp().drag_to(start_x + offset_x, start_y + offset_y);
                    }
                }
            ));
            widget.add_controller(drag);

            // Ctrl+wheel zooms about the pointer, a notch a level; a plain wheel goes on to the
            // scrolled window.
            let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
            wheel.connect_scroll(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |wheel, _, dy| {
                    let ctrl = wheel
                        .current_event_state()
                        .contains(gtk::gdk::ModifierType::CONTROL_MASK);
                    if !ctrl || dy == 0.0 {
                        return glib::Propagation::Proceed;
                    }
                    let anchor = wheel
                        .current_event()
                        .and_then(|event| event.position())
                        .and_then(|(x, y)| {
                            let native = doc.native()?;
                            let (dx, dy) = native.surface_transform();
                            let point = gtk::graphene::Point::new((x - dx) as f32, (y - dy) as f32);
                            let point = native.compute_point(&doc, &point)?;
                            Some((f64::from(point.x()), f64::from(point.y())))
                        });
                    let zoom = crate::strip::zoom_step(doc.zoom(), dy < 0.0);
                    doc.set_zoom(zoom, anchor);
                    glib::Propagation::Stop
                }
            ));
            widget.add_controller(wheel);

            // A touchpad's pinch, about its middle.
            let pinch = gtk::GestureZoom::new();
            pinch.connect_begin(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |_, _| doc.imp().pinched.set(Some(doc.zoom()))
            ));
            pinch.connect_scale_changed(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |pinch, scale| {
                    let Some(from) = doc.imp().pinched.get() else {
                        return;
                    };
                    doc.set_zoom(from * scale, pinch.bounding_box_center());
                }
            ));
            pinch.connect_end(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |_, _| doc.imp().pinched.set(None)
            ));
            widget.add_controller(pinch);

            // A link says where it goes when the pointer rests on it, and how to get there —
            // the target is otherwise invisible, since the text is what is drawn.
            widget.set_has_tooltip(true);
            widget.connect_query_tooltip(|doc, x, y, _, tooltip| {
                let Some(link) = doc.imp().link_under(f64::from(x), f64::from(y)) else {
                    return false;
                };
                tooltip.set_text(Some(&format!("{}\nCtrl+click to open", link.href)));
                true
            });

            // The right-click menu. A secondary press outside the selection moves the caret there
            // first, so the menu acts on what is under the pointer.
            let menu = gtk::PopoverMenu::from_model(Some(&super::context_menu_model()));
            menu.set_parent(&*widget);
            menu.set_has_arrow(false);
            menu.set_halign(gtk::Align::Start);
            let _ = self.menu.set(menu);
            let secondary = gtk::GestureClick::new();
            secondary.set_button(gtk::gdk::BUTTON_SECONDARY);
            secondary.connect_pressed(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |gesture, _, x, y| {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    doc.grab_focus();
                    doc.imp().open_menu(x, y);
                }
            ));
            widget.add_controller(secondary);

            // The spelling half of that menu: its own action group on this widget, because
            // the word it acts on is this widget's state (`spot`) rather than the window's.
            let spell = gtk::gio::SimpleActionGroup::new();
            let correct = gtk::gio::SimpleAction::new("correct", Some(glib::VariantTy::STRING));
            correct.connect_activate(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |_, with| {
                    if let Some(with) = with.and_then(gtk::glib::Variant::get::<String>) {
                        doc.imp().correct(&with);
                    }
                }
            ));
            spell.add_action(&correct);
            for (name, add) in [("ignore", false), ("add", true)] {
                let action = gtk::gio::SimpleAction::new(name, None);
                action.connect_activate(glib::clone!(
                    #[weak(rename_to = doc)]
                    widget,
                    move |_, _| doc.imp().keep(add)
                ));
                spell.add_action(&action);
            }
            let none = gtk::gio::SimpleAction::new("none", None);
            none.set_enabled(false);
            spell.add_action(&none);
            widget.insert_action_group("spell", Some(&spell));

            // The grammar half, the same way: the finding is this widget's (`grammar_spot`).
            let grammar = gtk::gio::SimpleActionGroup::new();
            let correct = gtk::gio::SimpleAction::new("correct", Some(glib::VariantTy::STRING));
            correct.connect_activate(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |_, with| {
                    if let Some(with) = with.and_then(gtk::glib::Variant::get::<String>) {
                        doc.imp().correct_grammar(&with);
                    }
                }
            ));
            grammar.add_action(&correct);
            let ignore = gtk::gio::SimpleAction::new("ignore", None);
            ignore.connect_activate(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |_, _| doc.imp().ignore_grammar_rule()
            ));
            grammar.add_action(&ignore);
            let none = gtk::gio::SimpleAction::new("none", None);
            none.set_enabled(false);
            grammar.add_action(&none);
            widget.insert_action_group("grammar", Some(&grammar));
        }
    }

    impl WidgetImpl for Doc {
        /// A scrollable asks for nothing and takes what it is given.
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            (0, 0, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            self.size_scrollbars(width, height);
            if let Some(page) = self.pending_page.take() {
                self.obj().show_page(page);
            }
        }

        fn realize(&self) {
            self.parent_realize();
            self.restyle();
        }

        /// A theme switch: every colour drawn round and over the pages is read again.
        fn system_setting_changed(&self, setting: &gtk::SystemSetting) {
            self.parent_system_setting_changed(setting);
            self.restyle();
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let (width, height) = (f64::from(widget.width()), f64::from(widget.height()));
            let palette = self.palette();
            let all = rect(0.0, 0.0, width, height);
            snapshot.append_color(&palette.background, &all);
            snapshot.append_color(&palette.desk, &all);
            let (Some(app), Some(shown)) = (self.app(), self.shown()) else {
                return;
            };
            let (left, top) = self.scrolled();
            snapshot.save();
            // Whole pixels, or every page would be resampled between two.
            snapshot.translate(&graphene::Point::new(
                -left.round() as f32,
                -top.round() as f32,
            ));
            let pages = shown.strip.visible(top, top + height);
            self.draw_pages(snapshot, &app, &shown, pages.clone());
            self.draw_marks(snapshot, &app, &shown, pages);
            snapshot.restore();
        }
    }

    impl ScrollableImpl for Doc {}

    impl Doc {
        pub fn app(&self) -> Option<Arc<App>> {
            self.app.borrow().clone()
        }

        /// How far the view is scrolled, across and down, in pixels.
        pub(super) fn scrolled(&self) -> (f64, f64) {
            let value = |adjustment: &RefCell<Option<gtk::Adjustment>>| {
                adjustment
                    .borrow()
                    .as_ref()
                    .map_or(0.0, gtk::prelude::AdjustmentExt::value)
            };
            (value(&self.hadjustment), value(&self.vadjustment))
        }

        fn palette(&self) -> Palette {
            if let Some(palette) = self.palette.get() {
                return palette;
            }
            let palette = Palette::of(&*self.obj());
            self.palette.set(Some(palette));
            palette
        }

        /// Drop everything derived from the style and derive it again.
        fn restyle(&self) {
            self.palette.set(None);
            self.obj().queue_allocate();
            self.obj().queue_draw();
        }

        /// The document's pages as they are now and where each sits in this window — rebuilt
        /// when the pages, the zoom or the width changed, and otherwise the same pair, so that
        /// a click and the paint before it read one layout.
        pub fn shown(&self) -> Option<Rc<Shown>> {
            let app = self.app()?;
            let set = self.session.document(&app);
            let window = f64::from(self.obj().width());
            let scale = f64::from(crate::print::pixels_per_point(self.zoom.get(), 1.0));
            let key = (window.to_bits(), scale.to_bits());
            if let Some(shown) = self.shown.borrow().as_ref()
                && Rc::ptr_eq(&shown.set, &set)
                && shown.key == key
            {
                return Some(shown.clone());
            }
            let sizes = set.pages.iter().map(|page| (page.width, page.height));
            let strip = Strip::new(sizes, scale, window);
            let shown = Rc::new(Shown { set, strip, key });
            self.shown.replace(Some(shown.clone()));
            Some(shown)
        }

        /// Each page in `pages` on its shadow, from its kept raster or a new one.
        fn draw_pages(
            &self,
            snapshot: &gtk::Snapshot,
            app: &App,
            shown: &Shown,
            pages: Range<usize>,
        ) {
            let palette = self.palette();
            let screen = f64::from(self.obj().scale_factor().max(1));
            // The screen's own pixels, so a HiDPI screen gets its extra ones as sharpness.
            let scale = crate::print::pixels_per_point(self.zoom.get(), screen);
            let setter = self.session.typesetter(app);
            let mut tiles = self.tiles.borrow_mut();
            // The pages in view and one either side, which a scroll reaches first.
            let keep = pages.start.saturating_sub(1)..pages.end + 1;
            tiles.retain(|number, _| keep.contains(number));
            for number in pages {
                let (Some(page), Some(sheet)) =
                    (shown.set.pages.get(number), shown.strip.sheet(number))
                else {
                    continue;
                };
                let area = rect(sheet.x, sheet.y, sheet.width, sheet.height);
                snapshot.append_outset_shadow(
                    &gsk::RoundedRect::from_rect(area, 0.0),
                    &palette.shadow,
                    0.0,
                    1.0,
                    0.0,
                    4.0,
                );
                let kept = tiles
                    .get(&number)
                    .is_some_and(|tile| tile.scale == scale.to_bits() && tile.page == *page);
                if !kept {
                    let raster = grind_print::raster::render(page, setter.fonts(), scale);
                    let texture = gdk::MemoryTexture::new(
                        raster.width as i32,
                        raster.height as i32,
                        gdk::MemoryFormat::R8g8b8a8Premultiplied,
                        &glib::Bytes::from_owned(raster.rgba),
                        raster.width as usize * 4,
                    );
                    tiles.insert(
                        number,
                        Tile {
                            page: page.clone(),
                            scale: scale.to_bits(),
                            texture: texture.upcast(),
                        },
                    );
                }
                if let Some(tile) = tiles.get(&number) {
                    snapshot.append_texture(&tile.texture, &area);
                }
            }
        }

        /// What is drawn over the pages and is not on them: the selection, spelling and grammar
        /// underlines, bookmark names and the caret.
        fn draw_marks(
            &self,
            snapshot: &gtk::Snapshot,
            app: &App,
            shown: &Shown,
            pages: Range<usize>,
        ) {
            let palette = self.palette();
            let (set, strip) = (&shown.set, &shown.strip);
            let caret = self.caret.get();
            let selection = self.selection();
            // A run of text as rectangles in the strip, one per line it crosses.
            let bands = |from: Caret, to: Caret| {
                place::selection_on(set, pages.clone(), from, to)
                    .into_iter()
                    .filter_map(|(number, band)| {
                        let (x, y) = strip.in_strip(number, band.x, band.y)?;
                        let scale = strip.scale();
                        Some((
                            x,
                            y,
                            f64::from(band.width) * scale,
                            f64::from(band.height) * scale,
                        ))
                    })
            };
            if let Some((from, to)) = selection {
                for (x, y, w, h) in bands(from, to) {
                    snapshot.append_color(&palette.selection, &rect(x, y, w, h));
                }
            }
            let Some(blocks) = blocks_on(set, pages.clone()) else {
                return;
            };
            let at = |block: usize, offset: usize| Caret { block, offset };

            // The word still being typed is not underlined: the caret at its end with nothing
            // selected is somebody halfway through it, and every word processor waits.
            if self.speller.borrow().is_some() {
                for wrong in app.misspellings(blocks.clone()) {
                    if selection.is_none() && wrong.being_typed(caret) {
                        continue;
                    }
                    let (from, to) = (wrong.offset, wrong.offset + wrong.len);
                    for (x, y, w, h) in bands(at(wrong.block, from), at(wrong.block, to)) {
                        squiggle(snapshot, x, x + w, y + h - 2.0, palette.misspelt);
                    }
                }
            }

            // A grammar finding: a wavy line in the accent for grammar and punctuation, a
            // dotted one in the dimmed ink for typography and style (`doc/grammar.md`
            // decision 2). An empty one (a missing comma) is marked across its neighbours. Not
            // one ending at the caret: that is still being typed.
            for flag in self.grammar_flags(blocks.clone()) {
                let range = &flag.finding.range;
                if selection.is_none() && flag.block == caret.block && range.end == caret.offset {
                    continue;
                }
                let (from, to) = if range.is_empty() {
                    (range.start.saturating_sub(1), range.end + 1)
                } else {
                    (range.start, range.end)
                };
                let quiet = matches!(
                    flag.finding.kind,
                    grind_text::grammar::Kind::Typography | grind_text::grammar::Kind::Style
                );
                for (x, y, w, h) in bands(at(flag.block, from), at(flag.block, to)) {
                    if quiet {
                        dotted(snapshot, x, x + w, y + h - 1.0, palette.on_paper);
                    } else {
                        squiggle(snapshot, x, x + w, y + h - 2.0, palette.grammar);
                    }
                }
            }

            // A bookmark contributes no characters, so it is the one part of a text document a
            // reader cannot see (`doc/view-modes.md` §3.6): a tick at its offset, and its name
            // past the end of its line, where it cannot cover the text.
            if self.names.get() {
                let viewport = app.get_viewport(blocks.clone());
                for index in blocks {
                    let Some(block) = viewport.get(index) else {
                        continue;
                    };
                    for (offset, name) in &block.marks {
                        let anchor = at(index, *offset);
                        let Some((x, y, h)) = spot_of(shown, anchor) else {
                            continue;
                        };
                        snapshot.append_color(&palette.on_paper, &rect(x - 1.0, y, 2.0, h));
                        let end = place::line_bounds(set, anchor)
                            .and_then(|(_, end)| spot_of(shown, end))
                            .map_or(x, |(x, ..)| x);
                        let label = self
                            .obj()
                            .create_pango_layout(Some(&format!("  \u{2039}{name}\u{203a}")));
                        snapshot.save();
                        snapshot.translate(&graphene::Point::new(end as f32, y as f32));
                        snapshot.append_layout(&label, &palette.on_paper);
                        snapshot.restore();
                    }
                }
            }

            let Some((x, y, h)) = spot_of(shown, caret) else {
                return;
            };
            // The input method's candidate window goes under the caret wherever a scroll or a
            // zoom put it, and this paint is where both have been settled.
            let (left, top) = self.scrolled();
            self.im.set_cursor_location(&gdk::Rectangle::new(
                (x - left) as i32,
                (y - top) as i32,
                1,
                h as i32,
            ));
            // ponytail: the text being composed is drawn over the page at the caret rather than
            // set in it, so it covers what follows until it is committed. Upgrade: the session
            // sets it as a pending fragment (`App::layout_composing`), `doc/page-model.md` PM5
            // item 5.
            if let Some((text, attrs)) = self.preedit.borrow().as_ref() {
                let layout = self.obj().create_pango_layout(Some(text));
                layout.set_attributes(Some(attrs));
                let mut font = gtk::pango::FontDescription::new();
                font.set_absolute_size(h * 0.7 * f64::from(gtk::pango::SCALE));
                layout.set_font_description(Some(&font));
                let (width, height) = layout.pixel_size();
                let below = y + (h - f64::from(height)) / 2.0;
                snapshot.append_color(&gdk::RGBA::WHITE, &rect(x, y, f64::from(width) + CARET, h));
                snapshot.save();
                snapshot.translate(&graphene::Point::new(x as f32, below as f32));
                snapshot.append_layout(&layout, &gdk::RGBA::BLACK);
                snapshot.restore();
                snapshot.append_color(&palette.accent, &rect(x + f64::from(width), y, CARET, h));
                return;
            }
            if selection.is_none() && self.obj().is_focus() && self.lit.get() {
                snapshot.append_color(&palette.accent, &rect(x, y, CARET, h));
            }
        }

        // --- input ---

        fn key_pressed(
            &self,
            keyval: gtk::gdk::Key,
            state: gtk::gdk::ModifierType,
        ) -> glib::Propagation {
            let mods = Mods {
                ctrl: state.contains(gtk::gdk::ModifierType::CONTROL_MASK),
                shift: state.contains(gtk::gdk::ModifierType::SHIFT_MASK),
            };
            // A modifier chord belongs to the window's actions (Ctrl+S, Ctrl+Z) unless the
            // map claims it, and Ctrl+Home is the one that is claimed.
            let Some(action) = keymap::action_for(key_of(keyval), mods) else {
                return glib::Propagation::Proceed;
            };
            match action {
                Action::Move(motion) => self.go(motion, mods.shift),
                Action::Split => self.split(),
                Action::EraseBack => self.erase_back(),
                Action::EraseForward => self.erase_forward(),
                Action::EraseWord(delta) => self.erase_word(delta),
                // Tab is structural where there is structure to change — it nests a list item, it
                // indents a block from its front — and a `text:tab` mid-sentence. Shift+Tab with
                // nothing to un-nest does nothing.
                Action::Indent(by) => {
                    if !self.indent(by) && by > 0 {
                        self.type_text("\t");
                    }
                }
            }
            glib::Propagation::Stop
        }

        /// Change the caret block's list depth, or make a list item of a block the caret is at the
        /// front of ([`grind_text::indent_kind`]). `false` when neither applied, leaving Tab to
        /// mean a tab character.
        pub fn indent(&self, by: i32) -> bool {
            let Some(app) = self.app() else { return false };
            let index = self.caret.get().block;
            let viewport = app.get_viewport(index..index + 1);
            let Some(block) = viewport.get(index) else {
                return false;
            };
            let Some(kind) = grind_text::indent_kind(&block.kind, self.caret.get().offset, by)
            else {
                return false;
            };
            if let Err(error) = app.set_kind(index, kind) {
                self.notice(error.to_string());
            }
            true
        }

        /// Every motion, routed to the core. `extend` is Shift: it grows the selection from
        /// wherever the caret was rather than replacing it, the way every other editor's
        /// Shift+arrow does.
        pub fn go(&self, motion: Motion, extend: bool) {
            let Some(app) = self.app() else { return };
            if app.block_count() == 0 {
                return;
            }
            let caret = self.caret.get();
            match extend {
                true if self.anchor.get().is_none() => self.anchor.set(Some(caret)),
                false => self.anchor.set(None),
                true => {}
            }
            match motion {
                Motion::Char(delta) => {
                    self.goal_x.set(None);
                    self.move_caret(self.stepped(&app, delta), true);
                }
                Motion::Word(delta) => {
                    self.goal_x.set(None);
                    self.move_caret(self.word_stepped(&app, delta), true);
                }
                Motion::Line(steps) | Motion::Page(steps) => {
                    let Some(shown) = self.shown() else { return };
                    let spot = place::locate(&shown.set, caret);
                    // A page is however many of the caret's lines fit, less one so that the line
                    // you were reading is still on screen after the jump.
                    let lines = match (motion, spot) {
                        (Motion::Page(_), Some(spot)) => {
                            let line = f64::from(spot.height) * shown.strip.scale();
                            ((f64::from(self.obj().height()) / line.max(1.0)) as isize - 1).max(1)
                        }
                        _ => 1,
                    };
                    // Remembered across a run of Down presses, which is what `goal_x` is for.
                    let goal = self.goal_x.get().or(spot.map(|spot| spot.x)).unwrap_or(0.0);
                    self.goal_x.set(Some(goal));
                    let delta = steps as isize * lines;
                    self.move_caret(place::step(&shown.set, caret, delta, goal), false);
                }
                Motion::LineStart | Motion::LineEnd => {
                    self.goal_x.set(None);
                    let bounds = self
                        .shown()
                        .and_then(|shown| place::line_bounds(&shown.set, caret));
                    if let Some((start, end)) = bounds {
                        self.move_caret(
                            match motion {
                                Motion::LineStart => start,
                                _ => end,
                            },
                            true,
                        );
                    }
                }
                Motion::DocStart => self.move_caret(grind_text::caret::START, true),
                Motion::DocEnd => self.move_caret(grind_text::caret::end(&app), true),
            }
        }

        /// One character left or right, rolling onto the neighbouring block at either end —
        /// `grind_text::caret::step`, which every shell's page shares.
        fn stepped(&self, app: &App, delta: i32) -> Caret {
            grind_text::caret::step(app, self.caret.get(), delta)
        }

        /// Where a word motion from the caret lands — `grind_text::caret::word`, the rule this
        /// window and the Mac share.
        fn word_stepped(&self, app: &App, delta: i32) -> Caret {
            grind_text::caret::word(app, self.caret.get(), delta)
        }

        fn block_len(&self, app: &App, index: usize) -> usize {
            grind_text::caret::block_len(app, index)
        }

        /// The caret a point in the widget means: the page under it, or the nearer of two, and
        /// on that page [`place::hit`] — a click in a margin is the text beside it.
        pub(super) fn caret_at(&self, x: f64, y: f64) -> Option<Caret> {
            let shown = self.shown()?;
            let (left, top) = self.scrolled();
            let (x, y) = (x + left, y + top);
            let page = shown.strip.page_at(y)?;
            let (x, y) = shown.strip.on_page(page, x, y)?;
            place::hit(&shown.set, page, x, y)
        }

        /// The press that starts a click or a drag — no telling which yet, so both are seeded
        /// alike. Shift extends the selection from the caret, as Shift+arrow does; a plain press
        /// anchors a fresh one here, which [`Doc::drag_to`] grows and which costs nothing when the
        /// button comes back up in place (an empty selection is [`Doc::selection`]'s none).
        pub fn click(&self, x: f64, y: f64, shift: bool) {
            let Some(caret) = self.caret_at(x, y) else {
                return;
            };
            match shift {
                true if self.anchor.get().is_none() => self.anchor.set(Some(self.caret.get())),
                false => self.anchor.set(Some(caret)),
                true => {}
            }
            self.move_caret(caret, true);
        }

        /// How many presses in a row this one makes: one more than the last when it came within
        /// the desktop's double-click time and a few pixels of it, and one otherwise.
        pub fn count_press(&self, time: u32, x: f64, y: f64) -> u8 {
            let settings = gtk::Settings::default();
            let window = settings
                .as_ref()
                .map_or(400, gtk::Settings::gtk_double_click_time)
                .max(1) as u32;
            let distance = f64::from(
                settings
                    .as_ref()
                    .map_or(5, gtk::Settings::gtk_double_click_distance)
                    .max(1),
            );
            let count = match self.presses.get() {
                Some((then, px, py, count))
                    if time.wrapping_sub(then) <= window
                        && (x - px).abs() <= distance
                        && (y - py).abs() <= distance =>
                {
                    // Past three it starts again at the word, the way a text field cycles.
                    count % 3 + 1
                }
                _ => 1,
            };
            self.presses.set(Some((time, x, y, count)));
            count
        }

        /// Select the word under a point — or, with `block`, its whole paragraph. The word is
        /// [`grind_text::word::around`]'s, so every shell that selects one agrees on where it
        /// ends.
        pub fn select_around(&self, x: f64, y: f64, block: bool) {
            let Some(app) = self.app() else { return };
            let Some(caret) = self.caret_at(x, y) else {
                return;
            };
            let text = app.input_text(caret.block).unwrap_or_default();
            let (start, end) = if block {
                (0, text.chars().count())
            } else {
                grind_text::word::around(&text, caret.offset)
            };
            self.anchor.set(Some(Caret {
                block: caret.block,
                offset: start,
            }));
            self.move_caret(
                Caret {
                    block: caret.block,
                    offset: end,
                },
                true,
            );
        }

        /// Open the right-click menu at a point, moving the caret there first unless the point
        /// is inside the selection — a right-click on selected text is about that text.
        fn open_menu(&self, x: f64, y: f64) {
            let Some(menu) = self.menu.get() else { return };
            // Over a misspelt word the menu leads with what it might have been. Asked for here,
            // one word at a time, because a suggestion costs tens of milliseconds.
            let spot = self.misspelling_at(x, y);
            let model = super::context_menu_model();
            if let (Some(wrong), Some(app)) = (&spot, self.app()) {
                let spelling = super::spelling_menu_model(&app.suggest(&wrong.word));
                model.insert_section(0, None, &spelling);
            }
            // A grammar finding under the point leads, ahead of a misspelling inside it.
            let finding = self.caret_at(x, y).and_then(|caret| self.grammar_at(caret));
            if let Some(flag) = &finding {
                model.insert_section(0, None, &super::grammar_menu_model(&flag.finding));
            }
            menu.set_menu_model(Some(&model));
            self.spot.replace(spot);
            self.grammar_spot.replace(finding);
            if let Some(caret) = self.caret_at(x, y) {
                let inside = self
                    .selection()
                    .is_some_and(|(from, to)| from <= caret && caret <= to);
                if !inside {
                    self.anchor.set(None);
                    self.move_caret(caret, true);
                }
            }
            menu.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            menu.popup();
        }

        /// The misspelt word under a point, if there is one.
        fn misspelling_at(&self, x: f64, y: f64) -> Option<grind_text::Misspelling> {
            self.speller.borrow().as_ref()?;
            let caret = self.caret_at(x, y)?;
            self.app()?
                .misspellings(caret.block..caret.block + 1)
                .into_iter()
                .find(|m| m.offset <= caret.offset && caret.offset <= m.offset + m.len)
        }

        /// Every grammar finding in `blocks` but the ignored rules'.
        pub fn grammar_flags(
            &self,
            blocks: std::ops::Range<usize>,
        ) -> Vec<grind_text::grammar::Flag> {
            let Some(app) = self.app() else {
                return Vec::new();
            };
            let ignored = self.grammar_ignored.borrow();
            app.grammar(blocks)
                .into_iter()
                .filter(|flag| !ignored.contains(&flag.finding.rule))
                .collect()
        }

        /// The grammar finding at a caret, if one covers it.
        pub fn grammar_at(&self, caret: Caret) -> Option<grind_text::grammar::Flag> {
            self.grammar_flags(caret.block..caret.block + 1)
                .into_iter()
                .find(|flag| {
                    flag.finding.range.start <= caret.offset
                        && caret.offset <= flag.finding.range.end
                })
        }

        /// Replace the finding the menu was opened on with `with` — one undo step, in the core,
        /// refused if its text has changed since.
        pub fn correct_grammar(&self, with: &str) {
            let Some(flag) = self.grammar_spot.take() else {
                return;
            };
            let Some(app) = self.app() else { return };
            let range = flag.finding.range.clone();
            let covered: String = app
                .get_viewport(flag.block..flag.block + 1)
                .get(flag.block)
                .map_or_default(|view| {
                    view.text
                        .chars()
                        .skip(range.start)
                        .take(range.len())
                        .collect()
                });
            match app.correct(flag.caret(), &covered, with) {
                Ok(()) => {
                    self.anchor.set(None);
                    self.move_caret(
                        Caret {
                            offset: range.start + with.chars().count(),
                            ..flag.caret()
                        },
                        true,
                    );
                }
                Err(error) => self.notice(error.to_string()),
            }
        }

        /// Stop drawing the rule of the finding the menu was opened on, and remember that.
        fn ignore_grammar_rule(&self) {
            let Some(flag) = self.grammar_spot.take() else {
                return;
            };
            let rule = flag.finding.rule;
            let mut setting = grind_grammar::setting::Setting::load();
            if !setting.ignored.contains(&rule) {
                setting.ignored.push(rule.clone());
                if let Err(error) = setting.save() {
                    self.notice(format!("Could not remember ignoring {rule}: {error}"));
                }
            }
            self.grammar_ignored.borrow_mut().push(rule);
            self.obj().queue_draw();
        }

        /// Replace the word the menu was opened on with `with` — one undo step, in the core.
        fn correct(&self, with: &str) {
            let Some(wrong) = self.spot.take() else {
                return;
            };
            let Some(app) = self.app() else { return };
            let at = Caret {
                block: wrong.block,
                offset: wrong.offset,
            };
            match app.correct(at, &wrong.word, with) {
                Ok(()) => {
                    self.anchor.set(None);
                    self.move_caret(
                        Caret {
                            offset: wrong.offset + with.chars().count(),
                            ..at
                        },
                        true,
                    );
                }
                Err(error) => self.notice(error.to_string()),
            }
        }

        /// Accept the word the menu was opened on: for this session (Ignore All), or in the
        /// person's own word list as well (Add to Dictionary).
        fn keep(&self, add: bool) {
            let Some(wrong) = self.spot.take() else {
                return;
            };
            let Some(speller) = self.speller.borrow().clone() else {
                return;
            };
            speller.accept(&wrong.word);
            if add && let Err(error) = grind_spell::personal::add(&wrong.word) {
                self.notice(format!(
                    "Could not add {:?} to your word list: {error}",
                    wrong.word
                ));
            }
            self.obj().queue_draw();
        }

        /// The link under a point, if the point is on one — its characters, not merely next to
        /// them, so a click just past a link's end is a click in the plain text after it.
        pub fn link_under(&self, x: f64, y: f64) -> Option<grind_text::Link> {
            let caret = self.caret_at(x, y)?;
            let link = self.app()?.link_at(caret).ok()??;
            (link.from <= caret && caret <= link.to).then_some(link)
        }

        /// Follow the link under a point, if there is one: tell the listeners its target.
        pub fn follow(&self, x: f64, y: f64) -> bool {
            let Some(link) = self.link_under(x, y) else {
                return false;
            };
            for hook in self.on_link.borrow().iter() {
                hook(link.href.clone());
            }
            true
        }

        pub fn caret_rect(&self) -> Option<gtk::gdk::Rectangle> {
            let (x, y, height) = self.point_of(self.caret.get())?;
            Some(gtk::gdk::Rectangle::new(
                x as i32,
                y as i32,
                1,
                height as i32,
            ))
        }

        /// Where a caret is drawn in the widget: its x, its top and its height, in pixels.
        /// `None` before the first allocation, or for a caret in text on no page.
        pub fn point_of(&self, caret: Caret) -> Option<(f64, f64, f64)> {
            if self.obj().width() == 0 {
                return None;
            }
            let (x, y, height) = spot_of(&*self.shown()?, caret)?;
            let (left, top) = self.scrolled();
            Some((x - left, y - top, height))
        }

        /// Dragging with the button down: the anchor [`Doc::click`] planted stays put and
        /// only the caret follows the pointer, which is what grows the highlighted band.
        pub fn drag_to(&self, x: f64, y: f64) {
            let Some(caret) = self.caret_at(x, y) else {
                return;
            };
            self.move_caret(caret, true);
        }

        // --- selection ---

        /// The selected range in document order. `None` both when nothing was selected and when
        /// the anchor and caret coincide again.
        pub fn selection(&self) -> Option<(Caret, Caret)> {
            let anchor = self.anchor.get()?;
            let caret = self.caret.get();
            (anchor != caret).then(|| (anchor.min(caret), anchor.max(caret)))
        }

        // --- the clipboard ---

        /// The selected text, blocks joined by a newline — a document has no character for a
        /// block boundary, and `\n` is what every other application means by one.
        pub fn selected_text(&self) -> Option<String> {
            let app = self.app()?;
            let (from, to) = self.selection()?;
            let mut out = String::new();
            for index in from.block..=to.block {
                let text = app.input_text(index).ok()?;
                let chars: Vec<char> = text.chars().collect();
                let start = if index == from.block { from.offset } else { 0 };
                let end = if index == to.block {
                    to.offset
                } else {
                    chars.len()
                };
                if index > from.block {
                    out.push('\n');
                }
                out.extend(&chars[start.min(chars.len())..end.min(chars.len())]);
            }
            Some(out)
        }

        /// Text in at the caret, replacing the selection; a newline splits a block, as pasting two
        /// paragraphs must. Pasted text is **not** read as markdown, unlike [`Doc::type_text`]:
        /// turning somebody's asterisks into bold would be editing what it was handed.
        pub fn paste_text(&self, text: &str) {
            let Some(app) = self.app() else { return };
            let mut caret = self.consume_selection(&app);
            for (index, line) in text.replace("\r\n", "\n").split('\n').enumerate() {
                if index > 0 {
                    if let Err(error) = app.split_block(caret) {
                        return self.notice(error.to_string());
                    }
                    caret = Caret {
                        block: caret.block + 1,
                        offset: 0,
                    };
                }
                if line.is_empty() {
                    continue;
                }
                match app.insert_text(caret, line) {
                    Ok(()) => caret.offset += line.chars().count(),
                    Err(error) => return self.notice(error.to_string()),
                }
            }
            self.move_caret(caret, true);
        }

        // --- editing ---

        /// If there is a selection, erase it and hand back the caret it collapses to —
        /// what typing a character, or Enter, over a selection does in every editor. Returns
        /// the caret unchanged when there is nothing selected.
        fn consume_selection(&self, app: &App) -> Caret {
            // The anchor goes whether or not it made a selection: a click plants one at the caret
            // for a drag to grow from, and left there it would turn the next character typed into
            // a selection the one after replaces (`a_click_then_two_keystrokes_types_both`).
            let selection = self.selection();
            self.anchor.set(None);
            let Some((from, to)) = selection else {
                return self.caret.get();
            };
            match app.erase(from, to) {
                Ok(_) => from,
                Err(error) => {
                    self.notice(error.to_string());
                    self.caret.get()
                }
            }
        }

        /// A typed character, read as markdown-shaped notation as it lands
        /// (`grind_text::markdown`): `**bold**` becomes bold and its markers go, `# ` makes a
        /// heading, three backticks fence a code paragraph. One `App::type_markdown` action, so one Ctrl+Z
        /// takes back the whole of `**bold**`.
        pub fn type_text(&self, text: &str) {
            let Some(app) = self.app() else { return };
            // A document with no blocks at all has nowhere to put a character, and the first
            // thing anybody does with an empty window is type into it.
            if app.block_count() == 0
                && let Err(error) = app.insert(0, BlockKind::Paragraph, "")
            {
                return self.notice(error.to_string());
            }
            let caret = self.consume_selection(&app);
            // Cloned out first: a `borrow()` in the scrutinee lives for the whole `match`, and
            // the arm below takes a `borrow_mut()` of the same cell.
            let resume = self.resume.borrow().clone();
            match app.type_markdown(caret, text, resume.as_ref()) {
                Ok(typed) => {
                    self.goal_x.set(None);
                    *self.resume.borrow_mut() = typed.resume;
                    self.place_caret(typed.caret, true);
                }
                Err(error) => self.notice(error.to_string()),
            }
        }

        pub fn split(&self) {
            let Some(app) = self.app() else { return };
            let caret = self.consume_selection(&app);
            match app.split_block(caret) {
                Ok(()) => self.move_caret(
                    Caret {
                        block: caret.block + 1,
                        offset: 0,
                    },
                    true,
                ),
                Err(error) => self.notice(error.to_string()),
            }
        }

        /// Backspace: erases the selection if there is one, otherwise the character before
        /// the caret — and at the front of a block the boundary itself, which is what
        /// [`App::erase`] across one already does.
        pub fn erase_back(&self) {
            let Some(app) = self.app() else { return };
            if let Some((from, to)) = self.selection() {
                self.anchor.set(None);
                match app.erase(from, to) {
                    Ok(_) => self.move_caret(from, true),
                    Err(error) => self.notice(error.to_string()),
                }
                return;
            }
            // No selection, but perhaps an anchor a click left at the caret — which an edit
            // must not leave behind, or the next one erases across it (`consume_selection`).
            self.anchor.set(None);
            self.erase_to(&app, self.stepped(&app, -1));
        }

        /// Erase from the caret to where a motion landed, and put the caret at the earlier end.
        fn erase_to(&self, app: &App, other: Caret) {
            let caret = self.caret.get();
            match grind_text::caret::erase_to(app, caret, other) {
                Ok(at) if at != caret => self.move_caret(at, true),
                Ok(_) => {}
                Err(error) => self.notice(error.to_string()),
            }
        }

        /// Ctrl+Backspace and Ctrl+Delete: erase as far as the word motion the same way would
        /// land — or, over a selection, just the selection, as the plain keys do.
        pub fn erase_word(&self, delta: i32) {
            let Some(app) = self.app() else { return };
            if self.selection().is_some() {
                return if delta > 0 {
                    self.erase_forward();
                } else {
                    self.erase_back();
                };
            }
            self.anchor.set(None);
            self.erase_to(&app, self.word_stepped(&app, delta));
        }

        pub fn erase_forward(&self) {
            let Some(app) = self.app() else { return };
            if let Some((from, to)) = self.selection() {
                self.anchor.set(None);
                match app.erase(from, to) {
                    Ok(_) => self.move_caret(from, true),
                    Err(error) => self.notice(error.to_string()),
                }
                return;
            }
            self.anchor.set(None);
            self.erase_to(&app, self.stepped(&app, 1));
        }

        // --- the caret ---

        /// Put the caret somewhere, and forget any style it was holding for the next character: a
        /// pending Bold belongs to where it was pressed. Typing keeps it, through
        /// [`Doc::place_caret`].
        pub fn move_caret(&self, caret: Caret, clear_goal: bool) {
            self.resume.replace(None);
            self.place_caret(caret, clear_goal);
        }

        /// The one place the caret changes: keep it inside the document, scroll it into view,
        /// tell the listeners, repaint. [`Doc::move_caret`] without forgetting the pending style.
        fn place_caret(&self, caret: Caret, clear_goal: bool) {
            self.caret.set(caret);
            if clear_goal {
                self.goal_x.set(None);
            }
            self.clamp_caret();
            self.scroll_into_view();
            self.announce();
            self.blink();
            self.obj().queue_draw();
            let caret = self.caret.get();
            for hook in self.on_moved.borrow().iter() {
                hook(caret);
            }
        }

        /// Show the caret and start it blinking again, as the desktop's settings say: a full
        /// blink every `gtk-cursor-blink-time`, and still after `gtk-cursor-blink-timeout`
        /// seconds of it, so an idle window stops repainting.
        pub fn blink(&self) {
            self.stop_blinking();
            self.lit.set(true);
            let Some(settings) = gtk::Settings::default() else {
                return;
            };
            if !settings.is_gtk_cursor_blink() || !self.obj().is_focus() {
                return;
            }
            let half = u64::try_from(settings.gtk_cursor_blink_time().max(200) / 2).unwrap_or(600);
            let turns = u64::try_from(settings.gtk_cursor_blink_timeout().max(1))
                .unwrap_or(10)
                .saturating_mul(1000)
                / half;
            let left = Cell::new(turns);
            let source = glib::timeout_add_local(
                std::time::Duration::from_millis(half),
                glib::clone!(
                    #[weak(rename_to = doc)]
                    self.obj(),
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move || {
                        let imp = doc.imp();
                        left.set(left.get().saturating_sub(1));
                        let done = left.get() == 0 && imp.lit.get();
                        imp.lit.set(!imp.lit.get() || done);
                        doc.queue_draw();
                        if done {
                            imp.blinking.replace(None);
                            return glib::ControlFlow::Break;
                        }
                        glib::ControlFlow::Continue
                    }
                ),
            );
            self.blinking.replace(Some(source));
        }

        pub fn stop_blinking(&self) {
            if let Some(source) = self.blinking.take() {
                source.remove();
            }
            self.lit.set(true);
        }

        /// History and edits move blocks around underneath the caret, so put it somewhere
        /// that exists.
        pub fn clamp_caret(&self) {
            let Some(app) = self.app() else { return };
            let mut caret = self.caret.get();
            caret.block = caret.block.min(app.block_count().saturating_sub(1));
            caret.offset = caret.offset.min(self.block_len(&app, caret.block));
            self.caret.set(caret);
        }

        pub(super) fn scroll_into_view(&self) {
            let widget = self.obj();
            // Before the first allocation there is no view to scroll into.
            if widget.width() == 0 || widget.height() == 0 {
                return;
            }
            // An edit that grew the document (Enter on the last line) is followed before the
            // next allocation sizes the scrollbar for it, and an adjustment clamps a value past
            // its old end — so the caret stopped a line short of the view. Size it now.
            self.size_scrollbars(widget.width(), widget.height());
            let Some(shown) = self.shown() else { return };
            let Some((x, y, height)) = spot_of(&shown, self.caret.get()) else {
                return;
            };
            let (across, down) = shown.strip.size();
            if let Some(adjustment) = self.vadjustment.borrow().as_ref() {
                let view = f64::from(widget.height());
                adjustment.set_value(strip::follow(adjustment.value(), view, down, y, height));
            }
            // A little room either side, so the caret is not on the window's very edge.
            if let Some(adjustment) = self.hadjustment.borrow().as_ref() {
                let view = f64::from(widget.width());
                let room = strip::GAP;
                let to = strip::follow(adjustment.value(), view, across, x - room, 2.0 * room);
                adjustment.set_value(to);
            }
        }

        /// Size both scrollbars to the strip of pages in a window this big.
        pub(super) fn size_scrollbars(&self, width: i32, height: i32) {
            let Some(shown) = self.shown() else { return };
            let (across, down) = shown.strip.size();
            // A line of twelve-point text, which is what a wheel notch is counted in.
            let line = 16.0 * shown.strip.scale();
            configure(
                self.hadjustment.borrow().as_ref(),
                f64::from(width).max(1.0),
                across,
                line,
            );
            configure(
                self.vadjustment.borrow().as_ref(),
                f64::from(height).max(1.0),
                down,
                line,
            );
        }

        /// The accessibility floor: a custom-drawn document has no other way to tell assistive
        /// technology the caret moved, so every move speaks the block's address and text.
        fn announce(&self) {
            let Some(app) = self.app() else { return };
            let caret = self.caret.get();
            let address = loc::format_offset(caret.block, caret.offset);
            let message = match app.input_text(caret.block) {
                Ok(text) if !text.is_empty() => format!("{address}: {text}"),
                _ => address,
            };
            if !crate::theme::heard(&*self.obj()) {
                return;
            }
            self.obj()
                .announce(&message, gtk::AccessibleAnnouncementPriority::Medium);
        }

        fn notice(&self, message: String) {
            for hook in self.on_notice.borrow().iter() {
                hook(message.clone());
            }
        }
    }

    /// The pages as set, and where each sits at one zoom in one window.
    pub struct Shown {
        pub set: Rc<grind_print::ops::Document>,
        pub strip: Strip,
        /// The window's width and the scale, as bits: what the strip was built for.
        key: (u64, u64),
    }

    /// One page's pixels, and the page and scale they were drawn from.
    pub struct Tile {
        page: grind_print::ops::Page,
        scale: u32,
        texture: gdk::Texture,
    }

    /// Where a caret is drawn in the strip: its x, its top and its height, in pixels.
    fn spot_of(shown: &Shown, caret: Caret) -> Option<(f64, f64, f64)> {
        let spot = place::locate(&shown.set, caret)?;
        let (x, y) = shown.strip.in_strip(spot.page, spot.x, spot.top)?;
        Some((x, y, f64::from(spot.height) * shown.strip.scale()))
    }

    /// The blocks with a line on `pages` — what a paint asks the core about. Table cells can
    /// put blocks out of order on a page, so it is the lowest to the highest.
    fn blocks_on(set: &grind_print::ops::Document, pages: Range<usize>) -> Option<Range<usize>> {
        let blocks = set
            .pages
            .get(pages)?
            .iter()
            .flat_map(|page| &page.lines)
            .map(|line| line.block);
        let (low, high) = blocks.fold(None, |range: Option<(usize, usize)>, block| {
            Some(range.map_or((block, block), |(low, high)| {
                (low.min(block), high.max(block))
            }))
        })?;
        Some(low..high + 1)
    }

    /// A wavy underline from `left` to `right` with its troughs on `y` — a misspelt word's mark,
    /// which every word processor draws this way so that it cannot be mistaken for underlined text.
    fn squiggle(snapshot: &gtk::Snapshot, left: f64, right: f64, y: f64, ink: gtk::gdk::RGBA) {
        const STEP: f32 = 2.0;
        if right - left < 1.0 {
            return;
        }
        let path = gtk::gsk::PathBuilder::new();
        let (left, right, y) = (left as f32, right as f32, y as f32);
        path.move_to(left, y);
        let mut at = left;
        let mut up = true;
        while at < right {
            at = (at + STEP).min(right);
            path.line_to(at, if up { y - STEP } else { y });
            up = !up;
        }
        snapshot.append_stroke(&path.to_path(), &gtk::gsk::Stroke::new(1.0), &ink);
    }

    /// A dotted underline from `left` to `right` on `y` — a typography or style finding's mark,
    /// quieter than a squiggle.
    fn dotted(snapshot: &gtk::Snapshot, left: f64, right: f64, y: f64, ink: gtk::gdk::RGBA) {
        let mut at = left;
        while at + 1.0 <= right {
            snapshot.append_color(&ink, &rect(at, y, 1.5, 1.5));
            at += 3.0;
        }
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> graphene::Rect {
        graphene::Rect::new(x as f32, y as f32, w as f32, h as f32)
    }

    /// Size a scrollbar. Unlike a spreadsheet's, a document has an end, so `upper` is
    /// simply how big it is.
    fn configure(adjustment: Option<&gtk::Adjustment>, page: f64, height: f64, step: f64) {
        let Some(adjustment) = adjustment else { return };
        let upper = height.max(page);
        adjustment.configure(
            adjustment.value().clamp(0.0, (upper - page).max(0.0)),
            0.0,
            upper,
            // A wheel notch is three lines, which is what every other document view does.
            step * 3.0,
            (page - step).max(step),
            page,
        );
    }

    /// A GDK keyval as [`keymap`] spells it. The keypad duplicates matter: a numeric-keypad
    /// arrow with Num Lock off is a different keyval and the same intent.
    fn key_of(keyval: gtk::gdk::Key) -> Key {
        use gtk::gdk::Key as K;
        match keyval {
            K::Left | K::KP_Left => Key::Left,
            K::Right | K::KP_Right => Key::Right,
            K::Up | K::KP_Up => Key::Up,
            K::Down | K::KP_Down => Key::Down,
            K::Home | K::KP_Home => Key::Home,
            K::End | K::KP_End => Key::End,
            K::Page_Up | K::KP_Page_Up => Key::PageUp,
            K::Page_Down | K::KP_Page_Down => Key::PageDown,
            K::Return | K::KP_Enter => Key::Return,
            // `ISO_Left_Tab` is what a keyboard sends for Shift+Tab, and a shell that only
            // matched `Tab` would leave Shift+Tab moving the focus out of the document.
            K::Tab | K::KP_Tab | K::ISO_Left_Tab => Key::Tab,
            K::BackSpace => Key::Backspace,
            K::Delete | K::KP_Delete => Key::Delete,
            _ => Key::Other,
        }
    }
}

/// Resolve a typed address — `p12`, `#intro`, `§2.1.3` — against the document as it is. `#intro`
/// and `§2.1` survive edits above them where a line number does not, which is why this window has
/// a Go to box.
pub fn caret_of(app: &App, address: &str) -> Result<Caret, String> {
    let loc = grind_text::loc::parse(address).map_err(|error| error.to_string())?;
    app.resolve_caret(&loc).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_print::place;
    use grind_text::BlockKind;

    /// **Every case that needs a widget, in one test, on one thread — on purpose.** GTK may be
    /// initialised once and used only from that thread, and the test harness gives every `#[test]`
    /// a thread of its own, even at `--test-threads=1`. So the cases are plain functions with this
    /// one entry point, each naming itself as it starts.
    ///
    /// **With no display it skips**, as in CI (`gtk.yml` installs GTK but no compositor).
    /// Everything decidable without one is in `strip.rs`, `keymap.rs` and `grind_print::place`.
    #[test]
    fn the_widget() {
        if gtk::init().is_err() {
            eprintln!("no display — skipping the widget cases");
            return;
        }
        for (name, case) in CASES {
            eprintln!("  widget case: {name}");
            case();
        }
    }

    /// The cases, in the order they run. A new one is added here and nowhere else — a
    /// function with `#[test]` on it would be a second thread and would take GTK down with it.
    const CASES: &[(&str, fn())] = &[
        (
            "typing, Enter and Backspace reach the document",
            typing_enter_and_backspace_reach_the_document,
        ),
        (
            "Shift extends a selection and typing replaces it",
            shift_extends_a_selection_and_typing_replaces_it,
        ),
        (
            "dragging the mouse selects text",
            dragging_the_mouse_selects_text,
        ),
        (
            "a click then two keystrokes types both",
            a_click_then_two_keystrokes_types_both,
        ),
        (
            "a double-click selects a word and a triple-click the paragraph",
            a_double_click_selects_a_word_and_a_triple_click_the_paragraph,
        ),
        (
            "a style pressed at a bare caret is what is typed next",
            a_style_pressed_at_a_bare_caret_is_what_is_typed_next,
        ),
        (
            "Ctrl+arrows move by word and Ctrl+Backspace erases one",
            ctrl_arrows_move_by_word_and_ctrl_backspace_erases_one,
        ),
        (
            "the find bar selects each hit in turn, ignoring case",
            the_find_bar_selects_each_hit_in_turn,
        ),
        (
            "the find bar replaces every exact occurrence",
            the_find_bar_replaces_every_exact_occurrence,
        ),
        (
            "a Title style is set on a taller line than the body",
            a_title_style_is_set_on_a_taller_line_than_the_body,
        ),
        (
            "a grammar finding is offered in the menu and corrected",
            a_grammar_finding_is_offered_in_the_menu_and_corrected,
        ),
        (
            "the paragraph after a picture starts below it",
            the_paragraph_after_a_picture_starts_below_it,
        ),
        (
            "Down moves by a wrapped line, not by a block",
            down_moves_by_a_wrapped_line_not_by_a_block,
        ),
        (
            "markdown as it is typed reaches the document",
            typing_markdown_formats_the_span,
        ),
        (
            "the pages are drawn on the desk, one under the other",
            the_pages_are_drawn_on_the_desk,
        ),
        (
            "a zoom keeps the point under the pointer where it was",
            a_zoom_keeps_the_point_under_the_pointer,
        ),
        (
            "Tab nests a list item and Shift+Tab ends the list",
            tab_nests_a_list_item_and_shift_tab_ends_the_list,
        ),
        (
            "the clipboard's two halves are plain text and blocks",
            the_clipboards_two_halves_are_plain_text_and_blocks,
        ),
        (
            "the code view shows the projection, tagged and marked",
            the_code_view_shows_the_projection,
        ),
        (
            "Enter on the last line scrolls the new one into view",
            enter_on_the_last_line_scrolls_the_new_one_into_view,
        ),
        (
            "a link is followed, and edited from Ctrl+K",
            a_link_is_followed_and_edited,
        ),
        (
            "the problems dialog builds from a document's findings",
            the_problems_dialog_builds,
        ),
    ];

    /// The findings dialog, built against a document with a finding — a skipped heading level.
    /// Here because every widget call in `lint.rs` needs the GTK thread, and pure tests cannot
    /// catch a dialog that panics on its first row.
    fn the_problems_dialog_builds() {
        let app = Arc::new(App::new());
        app.insert(0, BlockKind::Heading { level: 1 }, "One")
            .expect("inserts");
        app.insert(1, BlockKind::Heading { level: 3 }, "Too deep")
            .expect("inserts");
        let report = app.lint(&grind_text::lint::Options::default());
        assert!(
            report.diagnostics.iter().any(|d| d.rule == "heading-skip"),
            "the document really does have something to report: {:?}",
            report.diagnostics
        );

        let went = Rc::new(std::cell::RefCell::new(String::new()));
        let dialog = crate::lint::dialog(&app, move |address: &str| {
            went.borrow_mut().push_str(address);
        });
        assert_eq!(dialog.title().as_str(), "Check Document");
        assert!(dialog.child().is_some(), "it has content to show");
    }

    /// The other page of the window: the buffer holds the projection exactly, every token carries
    /// the tag the writer named it with, and the marked line is the one the cursor reports. Here
    /// because it needs a `gtk::TextView` and the GTK thread.
    fn the_code_view_shows_the_projection() {
        let app = Arc::new(App::new());
        app.insert(0, BlockKind::Heading { level: 1 }, "Addresses")
            .expect("inserts");
        app.insert(1, BlockKind::Paragraph, "A paragraph.")
            .expect("inserts");
        let projection = app.project();

        let view = crate::code::build();
        crate::code::fill(&view, &projection, projection.line_of("p2"));
        let buffer = view.buffer();
        let text = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string();
        assert_eq!(
            text,
            projection.text().trim_end_matches('\n'),
            "the buffer is the projection"
        );

        // The node name of the first line carries the writer's own tag, which is the whole of
        // "highlighting comes from the writer" made checkable.
        let at = buffer.iter_at_offset(text.find('h').expect("a heading node") as i32);
        assert!(
            at.has_tag(&buffer.tag_table().lookup("node").expect("the tag exists")),
            "`h` is tagged as a node"
        );

        // And the mark is where the caret's block is, both ways round.
        let line = projection.line_of("p2").expect("the paragraph is anchored");
        assert_eq!(crate::code::line_at_cursor(&view), line);
        crate::code::go_to(&view, 0);
        assert_eq!(crate::code::line_at_cursor(&view), 0);

        // `mark` must not move the cursor — it runs from the handler the cursor's move fired — so
        // marking a different line leaves the cursor where it was.
        crate::code::mark(&view, line);
        assert_eq!(
            crate::code::line_at_cursor(&view),
            0,
            "`mark` tags a line and moves nothing"
        );
    }

    /// A widget with a document in it and a size to lay it out at. Only ever called from
    /// [`the_widget`], which has already decided there is a display to build one on.
    fn shell(paragraphs: &[&str]) -> (Doc, Arc<App>) {
        let app = Arc::new(App::new());
        // The first paragraph goes into the one a new document already has (`Document::default`),
        // not in front of it.
        for (index, text) in paragraphs.iter().enumerate() {
            match index {
                0 => app.set_text(0, text).expect("sets"),
                _ => app
                    .insert(index, BlockKind::Paragraph, text)
                    .expect("inserts"),
            }
        }
        let doc = Doc::new(app.clone());
        // Unparented widgets have no size, and a caret motion measured at a width of zero
        // wraps every paragraph one character to the line.
        doc.allocate(600, 400, -1, None);
        (doc, app)
    }

    fn text(app: &App) -> String {
        app.get_viewport(0..app.block_count())
            .iter()
            .map(|block| block.text.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The shell's whole editing surface, driven the way the input method and the key
    /// controller drive it. What this proves is the wiring: every one of these is a core
    /// call, and getting the caret wrong afterwards is this file's own bug.
    fn typing_enter_and_backspace_reach_the_document() {
        let (doc, app) = shell(&["hello world"]);
        let imp = doc.imp();
        imp.move_caret(
            Caret {
                block: 0,
                offset: 5,
            },
            true,
        );
        imp.type_text(" there");
        assert_eq!(text(&app), "hello there world");
        assert_eq!(doc.caret().offset, 11, "the caret follows what was typed");

        imp.split();
        assert_eq!(text(&app), "hello there\n world");
        assert_eq!(
            doc.caret(),
            Caret {
                block: 1,
                offset: 0
            }
        );

        // Backspace at the front of a block joins it back onto the one above.
        imp.erase_back();
        assert_eq!(text(&app), "hello there world");
        assert_eq!(doc.caret().offset, 11);

        // And undo takes the three of them back out, in the core — this shell has no
        // history of its own and never will (doc/plan.md rule 2).
        for _ in 0..3 {
            assert!(app.undo());
        }
        assert_eq!(text(&app), "hello world");
    }

    /// Links: a Ctrl+click on one hands its target over (and a click beside it does not), and
    /// Ctrl+K links a selection, edits the link at a bare caret, inserts a target as its own text
    /// when there is neither, and removes one. How one is drawn is the raster's.
    fn a_link_is_followed_and_edited() {
        let (doc, app) = shell(&["read the spec today"]);
        let imp = doc.imp();
        let at = |offset| Caret { block: 0, offset };
        app.set_link(at(9), at(13), Some("https://x.org")).unwrap();
        doc.invalidate();

        // A point on "spec" is on the link; one on "read" is not.
        let followed = Rc::new(RefCell::new(Vec::new()));
        doc.connect_link(glib::clone!(
            #[strong]
            followed,
            move |href| followed.borrow_mut().push(href)
        ));
        let (x, y) = point_of(&doc, 10);
        assert!(imp.follow(x, y));
        let (x, y) = point_of(&doc, 1);
        assert!(!imp.follow(x, y));
        assert_eq!(*followed.borrow(), ["https://x.org"]);

        let editor = crate::link::Editor::new(&app, &doc);
        // A selection is what gets linked.
        doc.select(at(0), at(4));
        editor.prepare();
        editor.link_to("#top").unwrap();
        assert_eq!(app.link(at(0), at(4)).unwrap().as_deref(), Some("#top"));
        // A bare caret inside a link edits the whole of it.
        doc.go_to(at(11));
        editor.prepare();
        editor.link_to("https://y.org").unwrap();
        let edited = app.link_at(at(11)).unwrap().unwrap();
        assert_eq!((edited.from, edited.to), (at(9), at(13)));
        assert_eq!(edited.href, "https://y.org");
        // With neither, the target is inserted as its own linked text.
        doc.go_to(at(19));
        editor.prepare();
        editor.link_to("https://z").unwrap();
        assert_eq!(text(&app), "read the spec todayhttps://z");
        assert_eq!(
            app.link_at(at(25)).unwrap().map(|l| l.href).as_deref(),
            Some("https://z")
        );
        assert!(editor.link_to("").is_err(), "an empty target is refused");
        // And Remove takes the link at the caret away, leaving the text.
        doc.go_to(at(10));
        editor.prepare();
        editor.unlink();
        assert_eq!(app.link_at(at(10)).unwrap(), None);
        assert_eq!(text(&app), "read the spec todayhttps://z");
    }

    /// Enter at the bottom of the window keeps the caret on screen: the caret move must follow a
    /// layout with the new line in it, and the scrollbar's range the longer document.
    fn enter_on_the_last_line_scrolls_the_new_one_into_view() {
        let (doc, app) = shell(&["first"]);
        let adjustment = gtk::Adjustment::default();
        doc.set_vadjustment(Some(&adjustment));
        doc.allocate(600, 400, -1, None);
        let imp = doc.imp();
        for _ in 0..60 {
            let last = app.block_count() - 1;
            imp.move_caret(
                Caret {
                    block: last,
                    offset: 0,
                },
                true,
            );
            imp.split();
            // What the window's observer does once the edit has released the lock.
            doc.invalidate();
        }
        assert_eq!(app.block_count(), 61);
        assert!(adjustment.value() > 0.0, "the view scrolled at all");
        let (_, top, height) = imp.point_of(doc.caret()).expect("the caret is on a page");
        assert!(
            top >= 0.0 && top + height <= adjustment.page_size() + 0.5,
            "the caret's line ({top}..{}) is inside the view (..{})",
            top + height,
            adjustment.page_size()
        );
    }

    /// Shift+arrow grows a selection from wherever the caret started, and typing over one
    /// replaces it — the two behaviours every other editor's Shift key has, neither of which
    /// existed before this file grew an anchor.
    fn shift_extends_a_selection_and_typing_replaces_it() {
        let (doc, app) = shell(&["hello world"]);
        let imp = doc.imp();
        imp.move_caret(
            Caret {
                block: 0,
                offset: 0,
            },
            true,
        );
        assert_eq!(doc.selection(), None, "no selection yet");

        for _ in 0..5 {
            imp.go(crate::keymap::Motion::Char(1), true);
        }
        assert_eq!(
            doc.selection(),
            Some((
                Caret {
                    block: 0,
                    offset: 0
                },
                Caret {
                    block: 0,
                    offset: 5
                }
            )),
            "\"hello\" is selected"
        );

        // A plain (non-Shift) move drops the selection rather than replacing it with a new
        // one-character one.
        imp.go(crate::keymap::Motion::Char(1), false);
        assert_eq!(doc.selection(), None);

        // Re-select "hello" and type over it: the selection is erased first, same as every
        // other editor's Shift+arrow-then-type.
        imp.move_caret(
            Caret {
                block: 0,
                offset: 5,
            },
            true,
        );
        for _ in 0..5 {
            imp.go(crate::keymap::Motion::Char(-1), true);
        }
        imp.type_text("goodbye");
        assert_eq!(text(&app), "goodbye world");
        assert_eq!(doc.selection(), None, "typing collapses the selection");
        assert_eq!(doc.caret().offset, 7);
    }

    /// A mouse selects by dragging: the press plants the anchor, and the drag's own updates
    /// move the caret without disturbing it — the two halves `GestureDrag` was wired to
    /// drive, exercised here without one.
    fn dragging_the_mouse_selects_text() {
        let (doc, _app) = shell(&["hello world"]);
        let imp = doc.imp();
        let y = point_of(&doc, 0).1;
        let x_of = |offset: usize| point_of(&doc, offset).0;

        imp.click(x_of(0), y, false);
        assert_eq!(
            doc.selection(),
            None,
            "a press alone is not yet a selection"
        );

        imp.drag_to(x_of(5), y);
        assert_eq!(
            doc.selection(),
            Some((
                Caret {
                    block: 0,
                    offset: 0
                },
                Caret {
                    block: 0,
                    offset: 5
                }
            )),
            "dragging from before \"h\" to just past \"hello\" selects it"
        );

        // Dragging back past where the press started still reports one range in document
        // order, whichever end the pointer is actually over.
        imp.drag_to(x_of(0), y);
        assert_eq!(
            doc.selection(),
            None,
            "back where the press started, nothing is selected"
        );
    }

    /// Where `offset` in block 0 of a [`shell`] is on screen — the point a click there lands on.
    fn point_of(doc: &Doc, offset: usize) -> (f64, f64) {
        let (x, top, _) = doc
            .imp()
            .point_of(Caret { block: 0, offset })
            .expect("the block is on a page");
        (x, top + 1.0)
    }

    /// A click plants the selection's anchor at the caret for a drag to grow from; typing must
    /// clear it, or clicking and typing `xy` writes `y`, and two Backspaces after a click erase a
    /// character each side.
    fn a_click_then_two_keystrokes_types_both() {
        let (doc, app) = shell(&["hello world"]);
        let imp = doc.imp();
        let (x, y) = point_of(&doc, 5);
        imp.click(x, y, false);
        imp.type_text("x");
        imp.type_text("y");
        assert_eq!(text(&app), "helloxy world");
        assert_eq!(doc.selection(), None, "typing leaves nothing selected");

        let (x, y) = point_of(&doc, 5);
        imp.click(x, y, false);
        imp.erase_back();
        imp.erase_back();
        assert_eq!(text(&app), "helxy world", "both Backspaces erase backwards");
    }

    /// A double-click is the word under the pointer — [`grind_text::word::around`]'s — and a
    /// triple-click the whole paragraph, the way every text field on the desktop behaves.
    fn a_double_click_selects_a_word_and_a_triple_click_the_paragraph() {
        let (doc, _app) = shell(&["don't panic now"]);
        let imp = doc.imp();
        let at = |offset| Caret { block: 0, offset };
        let (x, y) = point_of(&doc, 8);
        imp.select_around(x, y, false);
        assert_eq!(doc.selection(), Some((at(6), at(11))), "`panic`");
        let (x, y) = point_of(&doc, 2);
        imp.select_around(x, y, false);
        assert_eq!(
            doc.selection(),
            Some((at(0), at(5))),
            "`don't`, apostrophe and all"
        );
        imp.select_around(x, y, true);
        assert_eq!(doc.selection(), Some((at(0), at(15))), "the paragraph");

        // And the press counting that tells the two apart: close in time and place counts up,
        // anything else starts again.
        assert_eq!(imp.count_press(1000, x, y), 1);
        assert_eq!(imp.count_press(1100, x + 1.0, y), 2);
        assert_eq!(imp.count_press(1200, x, y), 3);
        assert_eq!(
            imp.count_press(5000, x, y),
            1,
            "too late to be a double-click"
        );
    }

    /// Bold with nothing selected, then typing, is bold — the pending style
    /// ([`Doc::set_pending`]) is what the next character carries — and moving the caret first
    /// is changing one's mind, so it goes.
    fn a_style_pressed_at_a_bare_caret_is_what_is_typed_next() {
        let (doc, app) = shell(&["plain"]);
        let imp = doc.imp();
        let end = Caret {
            block: 0,
            offset: 5,
        };
        imp.move_caret(end, true);
        let mut bold = grind_text::CharStyle::default();
        bold.set_bold(true);
        doc.set_pending(bold.clone());
        assert_eq!(doc.pending(), Some(bold));
        imp.type_text("B");
        imp.type_text("b");
        let view = app.get_viewport(0..1);
        let run = view
            .get(0)
            .expect("the block")
            .runs
            .iter()
            .find(|run| run.text == "Bb")
            .expect("both typed characters are one run");
        assert!(run.props.is_bold(), "and it is bold");

        let mut italic = grind_text::CharStyle::default();
        italic.set_italic(true);
        doc.set_pending(italic);
        imp.move_caret(
            Caret {
                block: 0,
                offset: 0,
            },
            true,
        );
        assert_eq!(doc.pending(), None, "a caret that moves forgets it");
    }

    /// Ctrl+Right to the end of each word, across into the next block, and Ctrl+Backspace
    /// taking back exactly the word before the caret.
    fn ctrl_arrows_move_by_word_and_ctrl_backspace_erases_one() {
        use crate::keymap::Motion;
        let (doc, app) = shell(&["don't panic", "now"]);
        let imp = doc.imp();
        imp.move_caret(
            Caret {
                block: 0,
                offset: 0,
            },
            true,
        );
        let at = |block, offset| Caret { block, offset };
        let mut seen = Vec::new();
        for _ in 0..4 {
            imp.go(Motion::Word(1), false);
            seen.push(doc.caret());
        }
        assert_eq!(seen, [at(0, 5), at(0, 11), at(1, 0), at(1, 3)]);
        imp.go(Motion::Word(-1), true);
        assert_eq!(
            doc.selection(),
            Some((at(1, 0), at(1, 3))),
            "Shift extends by word"
        );

        doc.go_to(at(0, 11));
        imp.erase_word(-1);
        assert_eq!(text(&app), "don't \nnow");
    }

    /// Ctrl+F, typed lowercase, finds the capitalised word, and Enter walks on and wraps.
    fn the_find_bar_selects_each_hit_in_turn() {
        let (doc, app) = shell(&["Appendix one", "see the appendix", "APPENDIX"]);
        let find = crate::find::Find::new(&app, &doc);
        find.open();
        find.search("appendix");
        let at = |block, offset| Caret { block, offset };
        assert_eq!(doc.selection(), Some((at(0, 0), at(0, 8))));
        find.next();
        assert_eq!(doc.selection(), Some((at(1, 8), at(1, 16))));
        find.next();
        find.next();
        assert_eq!(doc.selection(), Some((at(0, 0), at(0, 8))), "wrapped");
    }

    /// Replace All writes exactly what was typed, over every block, and says how many changed.
    fn the_find_bar_replaces_every_exact_occurrence() {
        let (doc, app) = shell(&["tax and tax", "no match", "Tax"]);
        let find = crate::find::Find::new(&app, &doc);
        find.open();
        find.search("tax");
        find.replace_with("VAT");
        assert_eq!(text(&app), "VAT and VAT\nno match\nTax");
    }

    /// A `Title`-styled paragraph is set larger than the body — keyed off the block's *name*,
    /// because `Title` is `BlockKind::Paragraph` with nothing else to tell it apart.
    fn a_title_style_is_set_on_a_taller_line_than_the_body() {
        let (doc, app) = shell(&["Report", "body text"]);
        app.set_style(0..1, Some("Title".to_owned()))
            .expect("sets the style");
        doc.invalidate();
        let imp = doc.imp();
        let height = |block| {
            imp.point_of(Caret { block, offset: 0 })
                .expect("on a page")
                .2
        };
        assert!(
            height(0) > height(1),
            "a title reads larger than a plain paragraph"
        );
    }

    /// The built-in checker's finding under the caret leads the right-click menu with its message,
    /// its replacement and Ignore Rule; taking the replacement is one edit in the core, and an
    /// ignored rule is not drawn (`doc/grammar.md` §7).
    fn a_grammar_finding_is_offered_in_the_menu_and_corrected() {
        let (doc, app) = shell(&["I saw the the cat on the mat."]);
        app.set_checker(Some(std::sync::Arc::new(grind_grammar::Builtin::default())));
        let imp = doc.imp();
        let flag = imp
            .grammar_at(Caret {
                block: 0,
                offset: 11,
            })
            .expect("the finding under the caret");
        assert_eq!(flag.finding.rule, "builtin:repeated-word");
        let model = grammar_menu_model(&flag.finding);
        let actions: Vec<String> = (0..model.n_items())
            .flat_map(|section| {
                let items = model.item_link(section, "section").expect("a section");
                (0..items.n_items())
                    .filter_map(|i| {
                        items
                            .item_attribute_value(i, "action", Some(glib::VariantTy::STRING))
                            .and_then(|v| v.get::<String>())
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(
            actions,
            ["grammar.none", "grammar.correct", "grammar.ignore"]
        );
        doc.set_grammar_ignored(vec!["builtin:repeated-word".to_owned()]);
        assert!(
            imp.grammar_at(Caret {
                block: 0,
                offset: 11
            })
            .is_none(),
            "ignored"
        );
        doc.set_grammar_ignored(Vec::new());
        imp.grammar_spot.replace(Some(flag));
        imp.correct_grammar("");
        assert_eq!(
            app.get_viewport(0..1)
                .get(0)
                .map(|v| v.text.clone())
                .as_deref(),
            Some("I saw the cat on the mat.")
        );
    }

    /// A 4×4 red PNG — small enough to embed, and square, so a correct fit-to-column scale
    /// keeps its height equal to its width.
    const DOT_PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 4, 0, 0, 0, 4, 8, 2,
        0, 0, 0, 38, 147, 9, 41, 0, 0, 0, 16, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 0, 71,
        12, 196, 113, 0, 174, 147, 15, 241, 208, 95, 35, 158, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66,
        96, 130,
    ];

    /// A picture is one line as tall as itself, and the paragraph after it starts below it
    /// rather than over it.
    fn the_paragraph_after_a_picture_starts_below_it() {
        let (doc, app) = shell(&["", "after"]);
        let start = Caret {
            block: 0,
            offset: 0,
        };
        app.insert_image(start, "image/png".to_owned(), DOT_PNG.to_vec(), None, None)
            .expect("inserts");
        // No observer is wired up in this harness, so nothing tells the view on its own the
        // way a real window's bridge would.
        doc.invalidate();
        let imp = doc.imp();
        let (_, top, height) = imp.point_of(start).expect("the picture is placed");
        let (_, after, _) = imp
            .point_of(Caret {
                block: 1,
                offset: 0,
            })
            .expect("the paragraph is placed");
        assert!(height > 0.0);
        assert!(after >= top + height, "{after} is below {top} + {height}");
    }

    /// Down is the next *line*, not the next block — the line boxes the page was set with.
    fn down_moves_by_a_wrapped_line_not_by_a_block() {
        let long = "the cat sat on the mat and then it slept for a very long time indeed \
                    while the rain kept on falling outside the window all afternoon, and on \
                    into the evening, until the lamps were lit along the whole of the street";
        let (doc, app) = shell(&[long, "after"]);
        let imp = doc.imp();
        let shown = imp.shown().expect("the pages are set");
        let lines = shown.set.pages[0]
            .lines
            .iter()
            .filter(|line| line.block == 0)
            .count();
        assert!(
            lines > 1,
            "the fixture has to actually wrap or this proves nothing"
        );

        imp.go(crate::keymap::Motion::Line(1), false);
        assert_eq!(doc.caret().block, 0, "still inside the same paragraph");
        assert!(doc.caret().offset > 0, "but further down it");

        // Off the end of the last line, the same key carries into the next block — a
        // document is one flow, not a list of boxes.
        for _ in 1..lines {
            imp.go(crate::keymap::Motion::Line(1), false);
        }
        assert_eq!(doc.caret().block, 1);
        assert_eq!(app.block_count(), 2, "and nothing was edited on the way");

        // Home and End are the ends of the placed line.
        imp.go(crate::keymap::Motion::LineEnd, false);
        assert_eq!(doc.caret().offset, 5);
        let start = place::line_bounds(&shown.set, doc.caret()).map(|(start, _)| start);
        imp.go(crate::keymap::Motion::LineStart, false);
        assert_eq!(Some(doc.caret()), start);
    }

    /// The notation is `grind_text::markdown`'s and the edit is `App::type_markdown`'s, so
    /// what this checks is the *wiring*: a key that reaches `type_text` reaches the reading.
    fn typing_markdown_formats_the_span() {
        let (doc, app) = shell(&[""]);
        let imp = doc.imp();
        imp.move_caret(
            Caret {
                block: 0,
                offset: 0,
            },
            true,
        );
        for c in "say **this** now".chars() {
            imp.type_text(&c.to_string());
        }
        assert_eq!(text(&app), "say this now", "the markers are gone");
        let view = app.get_viewport(0..1);
        let bold = view
            .get(0)
            .expect("the block")
            .runs
            .iter()
            .find(|run| run.props.is_bold())
            .expect("a bold run");
        assert_eq!(bold.text, "this");
    }

    /// The view is pages on a desk: the second page starts below the first, a point in the
    /// gap between them is the nearer page's, and a paint draws without a panic.
    fn the_pages_are_drawn_on_the_desk() {
        let paragraph = "All work and no play makes a dull document. ".repeat(40);
        let paragraphs: Vec<&str> = (0..8).map(|_| paragraph.as_str()).collect();
        let (doc, app) = shell(&paragraphs);
        let imp = doc.imp();
        let shown = imp.shown().expect("the pages are set");
        assert!(shown.set.pages.len() >= 2, "the fixture spans pages");
        let (first, second) = (
            shown.strip.sheet(0).expect("a first page"),
            shown.strip.sheet(1).expect("a second page"),
        );
        assert!(second.y >= first.y + first.height + crate::strip::GAP);
        // Below the first page's last line, in its bottom margin: still the first page's text.
        let caret = imp
            .caret_at(first.x + 100.0, first.y + first.height - 2.0)
            .expect("a caret");
        assert!(caret.block < app.block_count());
        let spot = place::locate(&shown.set, caret).expect("placed");
        assert_eq!(spot.page, 0);

        let snapshot = gtk::Snapshot::new();
        gtk::subclass::prelude::WidgetImpl::snapshot(imp, &snapshot);
        assert!(snapshot.to_node().is_some(), "something was drawn");
        assert!(!imp.tiles.borrow().is_empty(), "and the page kept");

        // Text an input method is composing is drawn at the caret, uncommitted.
        imp.preedit
            .replace(Some(("かな".to_owned(), gtk::pango::AttrList::new())));
        let snapshot = gtk::Snapshot::new();
        gtk::subclass::prelude::WidgetImpl::snapshot(imp, &snapshot);
        assert!(snapshot.to_node().is_some());
        assert_eq!(app.input_text(0).unwrap(), paragraph, "and not written");
    }

    /// Zooming in about a point leaves the same spot of the same page under it, and the
    /// caret's line grows with the zoom.
    fn a_zoom_keeps_the_point_under_the_pointer() {
        let paragraph = "All work and no play makes a dull document. ".repeat(40);
        let paragraphs: Vec<&str> = (0..8).map(|_| paragraph.as_str()).collect();
        let (doc, _app) = shell(&paragraphs);
        let (across, down) = (gtk::Adjustment::default(), gtk::Adjustment::default());
        doc.set_hadjustment(Some(&across));
        doc.set_vadjustment(Some(&down));
        doc.allocate(600, 400, -1, None);
        let imp = doc.imp();
        down.set_value(300.0);
        let under = |x: f64, y: f64| {
            let shown = imp.shown().expect("set");
            let (left, top) = imp.scrolled();
            let page = shown.strip.page_at(y + top).expect("a page");
            (
                page,
                shown.strip.on_page(page, x + left, y + top).expect("on it"),
            )
        };
        let before = under(250.0, 150.0);
        let line = imp.point_of(doc.caret()).expect("placed").2;
        doc.set_zoom(2.0, Some((250.0, 150.0)));
        assert_eq!(doc.zoom(), 2.0);
        let after = under(250.0, 150.0);
        assert_eq!(before.0, after.0, "the same page");
        assert!((before.1.0 - after.1.0).abs() < 1.0 && (before.1.1 - after.1.1).abs() < 1.0);
        let grown = imp.point_of(doc.caret()).expect("placed").2;
        assert!(
            (grown - 2.0 * line).abs() < 0.5,
            "{line} at 100%, {grown} at 200%"
        );

        doc.zoom_step(false);
        assert!(doc.zoom() < 2.0);
        doc.set_zoom(99.0, None);
        assert_eq!(doc.zoom(), 4.0, "held inside the levels");

        // A whole page fits a 400-pixel-tall view, and the next is scrolled to the top.
        doc.fit_page();
        let sheet = imp.shown().expect("set").strip.sheet(0).expect("a page");
        assert!(sheet.height + 2.0 * crate::strip::PAD <= 400.5, "{sheet:?}");
        doc.show_page(1);
        let second = imp
            .shown()
            .expect("set")
            .strip
            .sheet(1)
            .expect("a second page");
        assert_eq!(down.value(), second.y - crate::strip::PAD);
    }

    /// Authoring a list, which this window could not do at all: Tab at the front of a
    /// paragraph starts one, Tab again nests it, and Shift+Tab out of the first level ends it.
    fn tab_nests_a_list_item_and_shift_tab_ends_the_list() {
        let (doc, app) = shell(&["an item"]);
        let imp = doc.imp();
        let kind = || {
            app.get_viewport(0..1)
                .get(0)
                .expect("the block")
                .kind
                .clone()
        };

        assert!(imp.indent(1), "the front of a paragraph starts a list");
        assert_eq!(kind(), BlockKind::ListItem { depth: 1 });
        assert!(imp.indent(1));
        assert_eq!(kind(), BlockKind::ListItem { depth: 2 });
        assert!(imp.indent(-1));
        assert_eq!(kind(), BlockKind::ListItem { depth: 1 });
        assert!(imp.indent(-1), "out of the first level");
        assert_eq!(
            kind(),
            BlockKind::Paragraph,
            "a list item at depth 0 is not one"
        );

        // Mid-word there is nothing structural to do, and the caller types a tab instead.
        imp.move_caret(
            Caret {
                block: 0,
                offset: 3,
            },
            true,
        );
        assert!(!imp.indent(1));
        assert_eq!(kind(), BlockKind::Paragraph);
    }

    /// The two halves either side of `gdk::Clipboard` — a selection as plain text, and plain text
    /// back into blocks; the clipboard itself is asynchronous and the display server's.
    fn the_clipboards_two_halves_are_plain_text_and_blocks() {
        let (doc, app) = shell(&["first line", "second line"]);
        let imp = doc.imp();
        imp.move_caret(
            Caret {
                block: 0,
                offset: 6,
            },
            true,
        );
        imp.anchor.set(Some(Caret {
            block: 0,
            offset: 6,
        }));
        imp.move_caret(
            Caret {
                block: 1,
                offset: 6,
            },
            true,
        );
        assert_eq!(
            imp.selected_text().as_deref(),
            Some("line\nsecond"),
            "across a block boundary, joined by the newline a block boundary is"
        );

        // And back: a newline is a block, not a character.
        let (doc, app2) = shell(&[""]);
        let imp = doc.imp();
        imp.paste_text("one\r\ntwo");
        assert_eq!(text(&app2), "one\ntwo");
        assert_eq!(
            app2.block_count(),
            2,
            "two paragraphs, not one with a break"
        );
        assert_eq!(doc.caret().offset, 3, "the caret is after what was pasted");
        // Nothing was read as markdown on the way in.
        let (doc, app3) = shell(&[""]);
        doc.imp().paste_text("**not bold**");
        assert_eq!(text(&app3), "**not bold**");
        drop(app);
    }

    #[test]
    fn an_address_resolves_to_a_caret_and_a_bad_one_says_so() {
        let app = App::new();
        app.insert(0, BlockKind::Paragraph, "hello")
            .expect("inserts");
        app.insert(1, BlockKind::Paragraph, "there")
            .expect("inserts");
        assert_eq!(
            caret_of(&app, "p2+3"),
            Ok(Caret {
                block: 1,
                offset: 3
            })
        );
        assert!(caret_of(&app, "nowhere").is_err());
    }
}
