// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The document view: a custom widget that draws blocks and a caret, and owns neither.
//!
//! `ui_sheet_gtk/src/grid.rs`'s counterpart, one document type over. Every paint asks
//! [`App::get_viewport`] for the blocks that fall on screen and [`App::layout_block`] for
//! their lines, draws them, and throws both away (doc/plan.md rule 1). The only state here is
//! presentation: where the caret is, and what column it is trying to keep while moving by
//! lines.
//!
//! **Not `GtkTextView`.** That widget owns a `GtkTextBuffer`, which is a second copy of the
//! document with its own undo stack, its own idea of what a paragraph is and no notion of a
//! `text:h` — rule 1's trap in its most tempting form. A widget drawing in `snapshot()` from
//! the core's own layout is more code once and keeps one document.
//!
//! **Where the editing model is, and is not.** Every motion is answered by the core:
//! Down-arrow is [`App::caret_line`], Home and End are [`App::caret_line_bounds`], a click is
//! [`grind_core::layout::Layout::offset_at`], typing is [`App::insert_text`], Enter is
//! [`App::split_block`], Backspace at the front of a block is what [`App::erase`] across a
//! boundary already does. This file decides *which* question to ask and where to draw the
//! answer — exactly the division `ui_tui/src/text/app.rs` makes, in a different unit, which
//! is what `doc/text-layout.md` chose Path C for.
//!
//! Typing goes through `GtkIMMulticontext`, so dead keys, compose sequences and input methods
//! work. [`crate::keymap`] never sees a printable character.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use libadwaita::gtk;
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::ObjectSubclassIsExt;

use grind_text::{App, Caret};
use gtk::glib;

use crate::geom::Flow;

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

    /// The document changed: forget everything measured from it and repaint.
    ///
    /// Called from the observer, so it covers this widget's own edits too — they reach the
    /// core the same way anything else does (doc/plan.md rule 3).
    pub fn invalidate(&self) {
        self.imp().flow.replace(None);
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

    /// Put the selection on the system clipboard. `false` when there is nothing selected.
    ///
    /// **Plain text, deliberately.** The clipboard this writes is the one every other
    /// application reads, and a run's `CharStyle` has no spelling in `text/plain` — the same
    /// answer `grind-web` gives, and the reason both shells' Copy is one line of arithmetic
    /// rather than a serialiser.
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

    /// Read the clipboard and put its text in at the caret, replacing the selection.
    ///
    /// Asynchronous because a clipboard read is: the bytes may be coming from another process
    /// that has not been asked for them yet, and GTK will not block for them.
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

/// The spelling section that leads the right-click menu over a misspelt word
/// (`doc/spelling.md`): what it might have been — each one `spell.correct` with the word as its
/// target — then Ignore All and Add to Dictionary. A free function for the reason
/// [`context_menu_model`] is.
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

/// The right-click menu on the page: what to do with the text under the pointer.
///
/// The clipboard, and nothing about the document as a whole — that is the primary menu's, the
/// same split `doc/sheet-shell.md`'s "Four surfaces" draws for the spreadsheet. A free function
/// returning the model so a test can walk it without a display.
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

    use grind_core::layout::Layout;
    use grind_text::{BlockKind, loc};
    use gtk::graphene;
    use gtk::pango;
    use gtk::subclass::prelude::*;

    use crate::geom::{self, Across};
    use crate::keymap::{self, Action, Key, Mods, Motion};
    use crate::metrics::{Face, Faces, run_attributes};
    use crate::theme::Palette;

    type NoticeHook = Box<dyn Fn(String)>;
    type MovedHook = Box<dyn Fn(Caret)>;

    /// How thick the caret is, and how far a list bullet sits left of its text.
    const CARET: f64 = 1.5;
    const BULLET_GAP: f64 = 14.0;

    pub struct Doc {
        pub app: RefCell<Option<Arc<App>>>,
        pub caret: Cell<Caret>,
        /// The other end of the selection, when there is one. `None` means "no selection",
        /// not "a selection at the caret" — see [`Doc::selection`], the one place that turns
        /// this and the caret into a range.
        pub anchor: Cell<Option<Caret>>,
        /// The column the caret is trying to keep while moving by lines — see
        /// [`App::caret_line`]. Cleared by any horizontal move, which is what makes walking
        /// down through a short line and out the other side come back where it started.
        pub goal_x: Cell<Option<f32>>,
        /// What `App::type_markdown` said the next character must be set in, so a notation
        /// ends where its closing marker does. Handed straight back and never read here.
        pub resume: RefCell<Option<grind_text::CharStyle>>,
        /// Every block's box, cached against the width it was measured at.
        ///
        /// ponytail: rebuilt from scratch whenever the document or the width changes, which
        /// lays out every block in the document rather than the ones that changed. Bounded by
        /// the document rather than by the screen, unlike everything else in this file; the
        /// upgrade path is a per-`BlockId` cache, and the reason not to have one yet is that
        /// it needs an invalidation rule the core does not hand out.
        pub flow: RefCell<Option<Placed>>,
        pub faces: RefCell<Option<Rc<Faces>>>,
        pub palette: Cell<Option<Palette>>,
        /// Whether the bookmark anchors are drawn — `doc/view-modes.md` §3.6. Presentation
        /// state like the caret beside it: a view mode is a **reading** of the document and
        /// never a change to it, so turning it off puts the page back exactly and there is
        /// nothing to save, undo or confirm.
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
        /// The last press — its time, where it was, and how many presses in a row it made — so
        /// a second press on the same spot is a double-click. Counted here rather than by a
        /// second gesture, because the drag that plants the caret and a click gesture that
        /// selects a word would both answer the same press, in an order GTK does not promise.
        pub presses: Cell<Option<(u32, f64, f64, u8)>>,
        /// The right-click menu, built once, parented on this widget and unparented in
        /// `dispose` — `ui_sheet_gtk`'s cell menu, for the same reason.
        pub menu: std::cell::OnceCell<gtk::PopoverMenu>,
        /// The dictionary the window attached to the `App`, kept here too so Ignore All and Add
        /// to Dictionary can teach it a word (`doc/spelling.md`). `None` draws no underlines.
        pub speller: RefCell<Option<Arc<grind_spell::Speller>>>,
        /// The misspelt word the right-click menu was opened on — what `spell.correct`,
        /// `spell.ignore` and `spell.add` act on.
        pub spot: RefCell<Option<grind_text::Misspelling>>,
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
                flow: RefCell::new(None),
                faces: RefCell::new(None),
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
                presses: Cell::new(None),
                menu: std::cell::OnceCell::new(),
                speller: RefCell::new(None),
                spot: RefCell::new(None),
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
        // The four properties `GtkScrollable` requires, overridden by hand for the reason
        // `ui_sheet_gtk/src/grid.rs` gives: the `Properties` derive's spelling has churned between
        // gtk4-rs releases and this shape does not move.
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
                "hadjustment" => {
                    self.hadjustment.replace(value.get().ok());
                }
                "vadjustment" => {
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
                    self.vadjustment.replace(adjustment);
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
                move |_| doc.imp().im.focus_in()
            ));
            focus.connect_leave(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |_| doc.imp().im.focus_out()
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

            // The right-click menu. A secondary press outside the selection moves the caret
            // there first, the way `ui_sheet_gtk`'s cell menu moves the selection: the menu
            // acts on what is under the pointer, and after this that is what is selected.
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
                    if let Some(with) = with.and_then(|v| v.get::<String>()) {
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
        }
    }

    impl WidgetImpl for Doc {
        /// A scrollable asks for nothing and takes what it is given.
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            (0, 0, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            // A resize re-wraps every paragraph, so the height the scrollbar is sized against
            // is only knowable here.
            self.size_scrollbar(width, height);
        }

        fn realize(&self) {
            self.parent_realize();
            self.restyle();
        }

        /// A theme switch or a font change: everything derived from the style goes, including
        /// every line break measured with the old font.
        fn system_setting_changed(&self, setting: &gtk::SystemSetting) {
            self.parent_system_setting_changed(setting);
            self.restyle();
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let width = f64::from(widget.width());
            let height = f64::from(widget.height());
            let Some(app) = self.app() else { return };
            let palette = self.palette();
            let faces = self.faces();
            let flow = self.flow(width);
            let (left, _) = geom::column(width);
            let scroll = self.scroll();

            snapshot.append_color(&palette.background, &rect(0.0, 0.0, width, height));

            // A table's rules, under everything: one rectangle per cell, drawn as four thin
            // bands rather than an outlined box, because a `gtk::Snapshot` has no stroke and
            // two neighbouring cells sharing an edge must not draw it twice at different
            // widths.
            for cell in flow.cells() {
                if cell.bottom() < scroll || cell.top > scroll + height {
                    continue;
                }
                let (x, y) = (left + cell.left, cell.top - scroll);
                for band in [
                    rect(x, y, cell.width, geom::RULE),
                    rect(x, y + cell.height - geom::RULE, cell.width, geom::RULE),
                    rect(x, y, geom::RULE, cell.height),
                    rect(x + cell.width - geom::RULE, y, geom::RULE, cell.height),
                ] {
                    snapshot.append_color(&palette.rule, &band);
                }
            }

            let slots = flow.visible(scroll, scroll + height);
            let Some((first, last)) = slots.first().zip(slots.last()) else {
                return;
            };
            // One read for the whole paint, exactly as wide as the screen (rule 1).
            let viewport = app.get_viewport(first.index..last.index + 1);
            let caret = self.caret.get();
            let selection = self.selection();
            // The word still being typed is not underlined: the caret at its end with nothing
            // selected is somebody halfway through it, and every word processor waits.
            let misspelt: Vec<grind_text::Misspelling> = match self.speller.borrow().is_some() {
                true => app
                    .misspellings(first.index..last.index + 1)
                    .into_iter()
                    .filter(|m| {
                        selection.is_some()
                            || !(m.block == caret.block && m.offset + m.len == caret.offset)
                    })
                    .collect(),
                false => Vec::new(),
            };
            let misspelt_ink = palette.misspelt;

            for slot in slots {
                let Some(block) = viewport.get(slot.index) else {
                    continue;
                };
                let x = left + slot.indent;
                let y = slot.top - scroll;

                // A block that is a picture — optionally with its caption's text — is drawn as
                // one rather than as the placeholder character `Run::Image::text()` returns
                // everywhere else. `doc/text-shell.md` has the rest of what a run that is not
                // text still cannot do (sit mid-sentence and lay out correctly, in particular).
                if let Some((image, caption)) = picture_of(block) {
                    let Some(texture) = texture_of(image) else {
                        continue;
                    };
                    let (w, h) = image_size(&texture, slot.width);
                    snapshot.append_texture(&texture, &rect(x, y, w, h));
                    if selection.is_none() && slot.index == caret.block && widget.is_focus() {
                        snapshot.append_color(&palette.accent, &rect(x, y, CARET, h));
                    }
                    if let Some(caption) = caption {
                        let caption_y = y + h + CAPTION_GAP;
                        draw_at(
                            snapshot,
                            faces.body().draw_wrapped(caption, slot.width),
                            x,
                            caption_y,
                            palette.dim,
                        );
                    }
                    continue;
                }

                let style = block.style.as_deref();
                let face = faces.of(&block.kind, style);
                let Ok(layout) = app.layout_block(slot.index, slot.width as f32, face) else {
                    continue;
                };
                let text: Vec<char> = block.text.chars().collect();
                let ink = match style {
                    Some("Subtitle") => palette.dim,
                    _ => palette.foreground,
                };

                if let Some(mark) = block.mark() {
                    // A list's mark is drawn rather than inserted: the character is not in the
                    // document, and putting one there would make it a character the caret
                    // could sit inside and a `p12+0` that means something else. Its number or
                    // its style's bullet (`BlockView::mark`), the same in every window; a
                    // label wider than a bullet ends where the bullet's room does.
                    let layout = face.draw(mark);
                    let wide = f64::from(layout.pixel_size().0);
                    draw_at(
                        snapshot,
                        layout,
                        x - BULLET_GAP.max(wide + BULLET_GAP / 3.0),
                        y,
                        palette.dim,
                    );
                }

                // The selection's band, one rectangle per line it crosses, drawn under the
                // text so a run painted over it stays legible.
                if let Some((start, end)) = selection
                    .and_then(|(from, to)| grind_text::paint::covered(slot.index, from, to))
                {
                    for line in layout.lines() {
                        if let Some((left, right)) =
                            grind_text::paint::band(&layout, line, start, end)
                        {
                            snapshot.append_color(
                                &palette.selection,
                                &rect(
                                    x + f64::from(left),
                                    y + f64::from(line.top),
                                    f64::from(right - left),
                                    f64::from(line.height),
                                ),
                            );
                        }
                    }
                }

                for line in layout.lines() {
                    let piece: String = text[line.start.min(text.len())..line.end.min(text.len())]
                        .iter()
                        .collect();
                    // A line's `end` includes the break that ended it, and a newline handed
                    // to Pango would start a second line inside this one.
                    let piece = piece.trim_end_matches('\n');
                    let attrs = run_attributes(
                        &block.runs,
                        line.start,
                        line.end,
                        piece,
                        face.size(),
                        palette.paper(),
                    );
                    draw_at(
                        snapshot,
                        face.draw_styled(piece, &attrs),
                        x,
                        y + f64::from(line.top),
                        ink,
                    );
                }

                // A misspelt word's wavy underline, under each line it crosses — `paint::band` is
                // the selection's own arithmetic, so a word that wraps is underlined on both
                // lines exactly where its characters are.
                for wrong in misspelt.iter().filter(|m| m.block == slot.index) {
                    for line in layout.lines() {
                        if let Some((left, right)) = grind_text::paint::band(
                            &layout,
                            line,
                            wrong.offset,
                            wrong.offset + wrong.len,
                        ) {
                            squiggle(
                                snapshot,
                                x + f64::from(left),
                                x + f64::from(right),
                                y + f64::from(line.top + line.height) - 2.0,
                                misspelt_ink,
                            );
                        }
                    }
                }

                // `doc/view-modes.md` §3.6: a bookmark is the named-range analogue and it
                // contributes no characters, which makes it the one part of a text document
                // a reader cannot see at all. This window can say *exactly* where one is —
                // it already has `x_at` for the caret — so the anchor gets a tick at its own
                // offset and the name is written at the end of the line it falls on, where
                // there is room for a word and where it cannot push the text along.
                if self.names.get() {
                    for (at, name) in &block.marks {
                        let line = layout.lines()[layout.line_at(*at)];
                        snapshot.append_color(
                            &palette.dim,
                            &rect(
                                x + f64::from(layout.x_at(*at)) - 1.0,
                                y + f64::from(line.top),
                                2.0,
                                f64::from(line.height),
                            ),
                        );
                        draw_at(
                            snapshot,
                            faces.body().draw(&format!("  \u{2039}{name}\u{203a}")),
                            x + f64::from(line.width),
                            y + f64::from(line.top),
                            palette.dim,
                        );
                    }
                }

                if selection.is_none() && slot.index == caret.block && widget.is_focus() {
                    let line = layout.lines()[layout.line_at(caret.offset)];
                    snapshot.append_color(
                        &palette.accent,
                        &rect(
                            x + f64::from(layout.x_at(caret.offset)),
                            y + f64::from(line.top),
                            CARET,
                            f64::from(line.height),
                        ),
                    );
                }
            }
        }
    }

    impl ScrollableImpl for Doc {}

    impl Doc {
        pub fn app(&self) -> Option<Arc<App>> {
            self.app.borrow().clone()
        }

        fn scroll(&self) -> f64 {
            self.vadjustment
                .borrow()
                .as_ref()
                .map_or(0.0, |a| a.value())
        }

        fn palette(&self) -> Palette {
            if let Some(palette) = self.palette.get() {
                return palette;
            }
            let palette = Palette::of(&*self.obj());
            self.palette.set(Some(palette));
            palette
        }

        pub fn faces(&self) -> Rc<Faces> {
            if let Some(faces) = self.faces.borrow().as_ref() {
                return faces.clone();
            }
            let faces = Rc::new(Faces::new(&self.obj().pango_context()));
            self.faces.replace(Some(faces.clone()));
            faces
        }

        /// Drop everything derived from the style and derive it again.
        fn restyle(&self) {
            self.palette.set(None);
            self.faces.replace(None);
            // Line breaking is font-dependent, so a new font is a new set of lines.
            self.flow.replace(None);
            self.obj().queue_allocate();
            self.obj().queue_draw();
        }

        /// Every block's box, measured at this width — cached, because a scroll must not
        /// re-lay-out the document.
        pub fn flow(&self, width: f64) -> Rc<Flow> {
            self.placed(width).flow
        }

        /// The flow and the table-cell measures it was built with, for a widget `width` wide —
        /// cached against the column they were measured at.
        fn placed(&self, width: f64) -> Placed {
            let (_, column) = geom::column(width);
            if let Some(placed) = self.flow.borrow().as_ref()
                && placed.column == column
            {
                return placed.clone();
            }
            let placed = self.place(column);
            self.flow.replace(Some(placed.clone()));
            placed
        }

        /// Stack every block at `column` — `grind_text::flow`, the one stacking every shell that
        /// draws a page shares, handed this window's numbers, its Pango faces and a way to size a
        /// picture.
        fn place(&self, column: f64) -> Placed {
            let Some(app) = self.app() else {
                return Placed {
                    column,
                    flow: Rc::new(Flow::new(geom::MARGIN, column)),
                    across: Rc::default(),
                };
            };
            // Before the faces, which read it: a cell's measure is a fact about the table's
            // shape and the column, and nothing about how tall anything is.
            let across = Rc::new(grind_text::flow::across(&app, column, &geom::SPACING));
            let faces = Column {
                faces: self.faces(),
                column,
                across: across.clone(),
            };
            let picture = |block: &grind_text::BlockView, width: f64| {
                picture_height(block, width, &faces.faces)
            };
            let flow = grind_text::flow::lay_out(&app, &faces, column, &geom::SPACING, &picture);
            Placed {
                column,
                flow: Rc::new(flow),
                across,
            }
        }

        /// One block's lines, measured in its own face — what a caret operation is asked in.
        ///
        /// Every caller needs the same three things and getting one of them from a different
        /// block's face would put the caret in the wrong place, so they are fetched together.
        pub fn measured(&self, index: usize) -> Option<(Layout, Rc<Faces>, BlockKind)> {
            let app = self.app()?;
            let viewport = app.get_viewport(index..index + 1);
            let block = viewport.get(index)?;
            let kind = block.kind.clone();
            let style = block.style.clone();
            let faces = self.faces();
            // The width this block was actually laid out at, which is its *cell's* inside a
            // table and the column less its indent everywhere else. Taken from the flow rather
            // than recomputed, so that a caret operation and the paint cannot disagree.
            let width = self.width_of(index, &kind) as f32;
            let layout = app
                .layout_block(index, width, faces.of(&kind, style.as_deref()))
                .ok()?;
            Some((layout, faces, kind))
        }

        /// The measure one block is laid out at: what [`Flow`] placed it with, and the column
        /// less its own indent when there is no flow yet (before the first allocation).
        fn width_of(&self, index: usize, kind: &BlockKind) -> f64 {
            let (_, column) = geom::column(f64::from(self.obj().width()));
            self.flow
                .borrow()
                .as_ref()
                .and_then(|placed| placed.flow.slot(index).map(|slot| slot.width))
                .unwrap_or_else(|| geom::SPACING.measure(kind, column))
        }

        /// How each block is set — this window's [`grind_text::Faces`], which is what every
        /// motion by line is asked through.
        ///
        /// Rebuilt per question rather than kept, because both halves of it change under the
        /// window: the faces on a theme change, the column on a resize.
        fn column(&self) -> Column {
            // The cached measures answer the width of a block inside a table cell, which no
            // rule about the block's own kind can. Cloned rather than rebuilt: a motion asks per
            // block, and rebuilding them per question would lay the document out once per
            // keystroke.
            let placed = self.placed(f64::from(self.obj().width()));
            Column {
                faces: self.faces(),
                column: placed.column,
                across: placed.across,
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
                // Tab is structural where there is structure to change and a `text:tab`
                // otherwise, which is the rule every word processor's Tab follows: it nests a
                // list item, it indents a block from its front, and in the middle of a
                // sentence it is a tab. Shift+Tab with nothing to un-nest does nothing rather
                // than typing anything — there is no such character.
                Action::Indent(by) => {
                    if !self.indent(by) && by > 0 {
                        self.type_text("\t");
                    }
                }
            }
            glib::Propagation::Stop
        }

        /// Change the caret block's list depth, or make a list item out of a block the caret
        /// is at the front of. `false` when neither applied, which is what leaves Tab free to
        /// mean a tab character.
        ///
        /// The rule itself is [`grind_text::indent_kind`] now — hoisted out of this file the
        /// day `grind-win32` wanted the same answer, so a list nests the same way in both
        /// windows.
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
            let faces = self.column();
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
                    // A page is however many body lines fit, less one so that the line you
                    // were reading is still on screen after the jump.
                    let lines = match motion {
                        Motion::Page(_) => {
                            let fit = f64::from(self.obj().height()) / self.faces().body().height();
                            (fit as isize - 1).max(1)
                        }
                        _ => 1,
                    };
                    let delta = steps as isize * lines;
                    // Remembered across a run of Down presses, which is what `goal_x` is for.
                    let goal = match self.goal_x.get() {
                        Some(x) => x,
                        None => app.caret_x(caret, &faces).unwrap_or(0.0),
                    };
                    self.goal_x.set(Some(goal));
                    if let Ok(moved) = app.caret_line(caret, delta, goal, &faces) {
                        self.move_caret(moved, false);
                    }
                }
                Motion::LineStart | Motion::LineEnd => {
                    self.goal_x.set(None);
                    if let Ok((start, end)) = app.caret_line_bounds(caret, &faces) {
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

        /// The block and offset a point in the widget lands on: which block the pointer is
        /// over, then which line, then where on it — the same two questions in the same
        /// order the core answers them, so a click and a Down-arrow land on the same offset.
        fn caret_at(&self, x: f64, y: f64) -> Option<Caret> {
            let width = f64::from(self.obj().width());
            let flow = self.flow(width);
            let (left, _) = geom::column(width);
            let scroll = self.scroll();
            // Both coordinates, measured from the column's left edge: inside a table the cells
            // of one row share a band of the page, so "which block" is a horizontal question
            // as well as a vertical one.
            grind_text::caret::hit(&flow, x - left, y + scroll, |index| {
                self.measured(index).map(|(layout, _, _)| layout)
            })
        }

        /// The press that starts either a click or a drag — there is no telling which yet,
        /// so both are seeded the same way. Shift extends the selection from wherever the
        /// caret already was, the way Shift+arrow does; a plain press starts a fresh one
        /// anchored here, which is what turns a subsequent [`Doc::drag_to`] into a mouse
        /// selection and costs nothing when the button just comes back up in place — a
        /// selection of zero characters is [`Doc::selection`]'s "no selection" case.
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
                .map_or(400, |s| s.gtk_double_click_time())
                .max(1) as u32;
            let distance = settings
                .as_ref()
                .map_or(5, |s| s.gtk_double_click_distance())
                .max(1) as f64;
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
            let (start, end) = match block {
                true => (0, text.chars().count()),
                false => grind_text::word::around(&text, caret.offset),
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
            menu.set_menu_model(Some(&model));
            self.spot.replace(spot);
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
            let widget = self.obj();
            if widget.width() == 0 {
                return None;
            }
            let flow = self.flow(f64::from(widget.width()));
            let caret = self.caret.get();
            let slot = flow.slot(caret.block)?;
            let (layout, _, _) = self.measured(caret.block)?;
            let line = layout.lines()[layout.line_at(caret.offset)];
            let (left, _) = geom::column(f64::from(widget.width()));
            let x = left + slot.indent + f64::from(layout.x_at(caret.offset));
            let y = slot.top - self.scroll() + f64::from(line.top);
            Some(gtk::gdk::Rectangle::new(
                x as i32,
                y as i32,
                1,
                f64::from(line.height) as i32,
            ))
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

        /// The selected range, normalised to document order. `None` covers both "nothing was
        /// ever selected" and "the anchor and the caret are back on top of each other" —
        /// a Shift+Right immediately followed by Shift+Left collapses a selection the same
        /// way letting go of Shift does, and every reader of this treats them alike.
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

        /// Text in, at the caret, replacing the selection. A newline splits a block, which is
        /// what pasting two paragraphs has to mean in a model whose blocks are the paragraphs.
        ///
        /// Pasted text is **not** read as markdown ([`Doc::type_text`] is): text arriving from
        /// a clipboard is text, and turning somebody's asterisks into bold is this program
        /// editing what it was handed. `grind-web` draws the same line in the same place.
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
            // The anchor goes whether or not it made a selection. A click plants one *at* the
            // caret, so that a drag has somewhere to grow from, and an edit that left it there
            // turned the character it just typed into a selection — the next keystroke then
            // replaced it, and clicking and typing `xy` wrote `y`. Found by a screenshot, and
            // pinned by `a_click_then_two_keystrokes_types_both`.
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

        /// A typed character, read as **markdown-shaped notation** as it lands
        /// (`grind_text::markdown`): `**bold**` becomes bold and its markers go, `# ` makes
        /// the block a heading, ``` fences a code paragraph.
        ///
        /// The reading is `App::type_markdown`'s, so this window, the browser, the terminal
        /// and the CLI agree about what `**` means — and it is one action, so one Ctrl+Z takes
        /// back the whole of `**bold**`. The toolbar is still the other way to say it, over a
        /// selection; this is the way that needs no pointer.
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
            let caret = self.caret.get();
            let from = self.stepped(&app, -1);
            if from == caret {
                return;
            }
            match app.erase(from, caret) {
                Ok(_) => self.move_caret(from, true),
                Err(error) => self.notice(error.to_string()),
            }
        }

        /// Ctrl+Backspace and Ctrl+Delete: erase as far as the word motion the same way would
        /// land — or, over a selection, just the selection, as the plain keys do.
        pub fn erase_word(&self, delta: i32) {
            let Some(app) = self.app() else { return };
            if self.selection().is_some() {
                return match delta > 0 {
                    true => self.erase_forward(),
                    false => self.erase_back(),
                };
            }
            self.anchor.set(None);
            let caret = self.caret.get();
            let target = self.word_stepped(&app, delta);
            let (from, to) = (caret.min(target), caret.max(target));
            if from == to {
                return;
            }
            match app.erase(from, to) {
                Ok(_) => self.move_caret(from, true),
                Err(error) => self.notice(error.to_string()),
            }
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
            let caret = self.caret.get();
            let to = match caret.offset < self.block_len(&app, caret.block) {
                true => Caret {
                    block: caret.block,
                    offset: caret.offset + 1,
                },
                false if caret.block + 1 < app.block_count() => Caret {
                    block: caret.block + 1,
                    offset: 0,
                },
                false => return,
            };
            if let Err(error) = app.erase(caret, to) {
                self.notice(error.to_string());
            }
        }

        // --- the caret ---

        /// The one place the caret changes: keep it inside the document, scroll it into
        /// view, tell the listeners, repaint.
        /// Put the caret somewhere, and forget any style it was holding for the next
        /// character — a pending Bold pressed with nothing selected belongs to *where* it was
        /// pressed, and clicking elsewhere is changing one's mind. Typing is the one move that
        /// keeps it, and it goes through [`Doc::place_caret`] to say so.
        pub fn move_caret(&self, caret: Caret, clear_goal: bool) {
            self.resume.replace(None);
            self.place_caret(caret, clear_goal);
        }

        /// [`Doc::move_caret`] without forgetting the pending style.
        fn place_caret(&self, caret: Caret, clear_goal: bool) {
            self.caret.set(caret);
            if clear_goal {
                self.goal_x.set(None);
            }
            self.clamp_caret();
            self.scroll_into_view();
            self.announce();
            self.obj().queue_draw();
            let caret = self.caret.get();
            for hook in self.on_moved.borrow().iter() {
                hook(caret);
            }
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
            let flow = self.flow(f64::from(widget.width()));
            let caret = self.caret.get();
            let Some(slot) = flow.slot(caret.block) else {
                return;
            };
            let Some((layout, _, _)) = self.measured(caret.block) else {
                return;
            };
            let line = layout.lines()[layout.line_at(caret.offset)];
            let target = (slot.top + f64::from(line.top), f64::from(line.height));
            // An edit that grew the document (Enter on the last line) is followed before the
            // next allocation sizes the scrollbar for it, and an adjustment clamps a value past
            // its old end — so the caret stopped a line short of the view. Size it now.
            self.size_scrollbar(widget.width(), widget.height());
            let Some(adjustment) = self.vadjustment.borrow().clone() else {
                return;
            };
            let page = f64::from(widget.height());
            adjustment.set_value(flow.follow(adjustment.value(), page, target));
        }

        /// Size the scrollbar to the document as it is now laid out at `width`.
        fn size_scrollbar(&self, width: i32, height: i32) {
            let flow = self.flow(f64::from(width));
            configure(
                self.vadjustment.borrow().as_ref(),
                f64::from(height).max(1.0),
                flow.height() + geom::MARGIN,
                self.faces().body().height(),
            );
        }

        /// The a11y floor (`doc/sheet-shell.md`, M9): a custom-drawn document has no other way
        /// to tell assistive technology that the caret moved, so every move speaks the
        /// block's address and its text.
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

    /// Whether a block is a picture, optionally followed by its caption's plain text.
    ///
    /// `grind_text::picture_of`'s own doc comment has the rest; this is a re-export under this
    /// module's own name because it started here and `grind-win32` wanted the same answer.
    pub(super) fn picture_of(
        block: &grind_text::BlockView,
    ) -> Option<(&grind_text::ImageView, Option<&str>)> {
        grind_text::picture_of(block)
    }

    /// Decode an embedded image's bytes into something [`gtk::Snapshot`] can paint. `None` for
    /// anything gdk-pixbuf has no loader for — a corrupt file, a format nobody installed —
    /// which is not a reason to refuse the rest of the document (§9's tolerance, over a
    /// picture instead of an XML element).
    ///
    /// ponytail: decodes on every repaint rather than caching the texture, so a document with
    /// a large image pays for it on every cursor blink. Worth a cache keyed by `BlockId`,
    /// invalidated the way `Doc::flow` already is, once a document with more than one real
    /// image makes the cost visible.
    pub(super) fn texture_of(image: &grind_text::ImageView) -> Option<gtk::gdk::Texture> {
        gtk::gdk::Texture::from_bytes(&glib::Bytes::from(&image.data)).ok()
    }

    /// How big to draw a texture: fit inside the column, keeping its aspect ratio, and never
    /// larger than its own pixels. ODF's own `svg:width`/`svg:height` are not used for this —
    /// turning a length like `13.229cm` into device pixels needs a resolution this shell does
    /// not otherwise track, and "fit the column" is the same default a simple viewer takes.
    pub(super) fn image_size(texture: &gtk::gdk::Texture, column: f64) -> (f64, f64) {
        let (w, h) = (
            f64::from(texture.width()).max(1.0),
            f64::from(texture.height()).max(1.0),
        );
        let width = w.min(column.max(1.0));
        (width, h * (width / w))
    }

    /// The gap between a picture and its caption — small, since the two read as one figure.
    const CAPTION_GAP: f64 = 4.0;

    /// How tall a caption's text comes out, wrapped to the column — measured with the same
    /// layout it will later be drawn with (`Face::draw_wrapped`), so the flow's reserved space
    /// and the paint always agree.
    pub(super) fn caption_height(face: &Face, text: &str, width: f64) -> f64 {
        f64::from(face.draw_wrapped(text, width).pixel_size().1)
    }

    /// The measure and the face of **every** block — this shell's [`grind_text::Faces`].
    ///
    /// Both halves are this window's own arithmetic and neither is uniform: a heading is set
    /// larger than the paragraph under it, and a list item's indent comes out of the column, so
    /// it is measured narrower. That is why the core asks per block rather than being handed
    /// one width and one provider for a whole motion — Down-arrow out of a heading used to
    /// measure the paragraph below it with the heading's font, and landed a few characters from
    /// where a click on the same spot would have.
    pub(super) struct Column {
        faces: Rc<Faces>,
        /// The text column's width in pixels, before any indent comes out of it.
        column: f64,
        /// Where every block in a table cell sits across the column
        /// (`grind_text::flow::across`) — the only thing that knows a block is in a cell and
        /// therefore measured narrower than the column. The flow was built through this same
        /// `Faces`, so a caret motion and the paint cannot disagree about a cell's width.
        across: Rc<HashMap<usize, Across>>,
    }

    impl grind_text::Faces for Column {
        fn of(
            &self,
            index: usize,
            kind: &BlockKind,
            style: Option<&str>,
        ) -> (f32, &dyn grind_text::Metrics) {
            let width = match self.across.get(&index) {
                Some(across) => across.width,
                None => geom::SPACING.measure(kind, self.column),
            };
            (width as f32, self.faces.of(kind, style))
        }
    }

    /// The flow at one column width, and the table-cell measures it was built with — cached
    /// together, because a caret motion reads the one and a paint the other and both have to be
    /// the same build.
    #[derive(Clone)]
    pub struct Placed {
        column: f64,
        flow: Rc<Flow>,
        across: Rc<HashMap<usize, Across>>,
    }

    /// How tall a picture block comes out at `width` — the picture fitted to it and its
    /// caption's lines under it — or `None` when the block is not a picture or its bytes will not
    /// decode, in which case `grind_text::flow` measures it as text.
    fn picture_height(block: &grind_text::BlockView, width: f64, faces: &Faces) -> Option<f64> {
        let (image, caption) = picture_of(block)?;
        let picture = image_size(&texture_of(image)?, width).1;
        Some(match caption {
            Some(caption) => picture + CAPTION_GAP + caption_height(faces.body(), caption, width),
            None => picture,
        })
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

    fn draw_at(
        snapshot: &gtk::Snapshot,
        layout: &pango::Layout,
        x: f64,
        y: f64,
        color: gtk::gdk::RGBA,
    ) {
        snapshot.save();
        snapshot.translate(&graphene::Point::new(x as f32, y as f32));
        snapshot.append_layout(layout, &color);
        snapshot.restore();
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> graphene::Rect {
        graphene::Rect::new(x as f32, y as f32, w as f32, h as f32)
    }

    /// Size the scrollbar. Unlike a spreadsheet's, a document has an end, so `upper` is
    /// simply how tall it is.
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

/// Resolve an address a user typed — `p12`, `#intro`, `§2.1.3` — against the document as it
/// now is.
///
/// The addressing no word processor's UI offers, and the reason this shell has a "Go to"
/// entry at all: `#intro` and `§2.1` survive edits above them where a line number does not.
pub fn caret_of(app: &App, address: &str) -> Result<Caret, String> {
    let loc = grind_text::loc::parse(address).map_err(|error| error.to_string())?;
    app.resolve_caret(&loc).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom;
    use grind_text::BlockKind;
    use imp::{caption_height, image_size, picture_of, texture_of};

    /// **Every case that needs a widget, in one test, on one thread — on purpose.**
    ///
    /// GTK may be initialised exactly once, and only ever used from the thread that did it.
    /// Rust's test harness gives every `#[test]` a thread of its own — *even at
    /// `--test-threads=1`* — so seven `#[test]`s over a widget is one that runs and six that
    /// panic with "Attempted to initialize GTK from two different threads". That is a property
    /// of the harness rather than of anything here, and it is what turned the `gtk` CI job red.
    ///
    /// So the cases below are ordinary functions and this is their only entry point. Each
    /// names itself as it starts, because a panic inside one would otherwise only say which
    /// *test* failed, and there is now exactly one.
    ///
    /// **Where there is no display it skips**, which is where CI runs
    /// (`.github/workflows/gtk.yml` installs the GTK packages and no compositor, on purpose).
    /// Everything decidable without a display is in `geom.rs`, `keymap.rs` and `imp`'s own
    /// tests, and runs unconditionally.
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
            "a Title style is drawn in a larger face than the body",
            a_title_style_is_drawn_in_a_larger_face_than_the_body,
        ),
        (
            "a picture with a caption reserves room for both",
            a_picture_with_a_caption_reserves_room_for_both,
        ),
        (
            "a block that is only an image is sized from the picture",
            a_block_that_is_only_an_image_is_sized_from_the_picture_not_a_line_of_text,
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
            "code is measured and drawn in a monospace face",
            code_is_measured_and_drawn_in_a_monospace_face,
        ),
        (
            "a run's colour, highlight and size are drawn",
            a_runs_colour_highlight_and_size_are_drawn,
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
            "a link is drawn as one, followed, and edited from Ctrl+K",
            a_link_is_drawn_followed_and_edited,
        ),
        (
            "the problems dialog builds from a document's findings",
            the_problems_dialog_builds,
        ),
    ];

    /// **D6** (`doc/dsl.md` §4.3). The findings dialog, built against a document that really has
    /// one — a heading level skipped, which is the word processor's own first rule.
    ///
    /// It is here for `the_code_view_shows_the_projection`'s reason: every widget call in
    /// `lint.rs` — the builders, `add_prefix`, `connect_activated` — needs a display and the one
    /// thread GTK was initialised on, and a table of pure functions cannot catch a dialog that
    /// panics on its first row. `ui_sheet_gtk/src/lint.rs` is the same file with `a1` where this
    /// one has `loc`, so this covers the shape of both.
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
        let dialog = crate::lint::dialog(&app, {
            let went = went.clone();
            move |address: &str| went.borrow_mut().push_str(address)
        });
        assert_eq!(dialog.title().as_str(), "Check Document");
        assert!(dialog.child().is_some(), "it has content to show");
    }

    /// **D9** (`doc/dsl.md` §6). The other page of the window: the buffer holds the projection
    /// exactly, every token carries the tag the *writer* named it with, and the marked line is
    /// the one the cursor reports back.
    ///
    /// It is here rather than in `code.rs` because it needs a `gtk::TextView`, and a widget
    /// needs the one thread that initialised GTK — which is what this whole harness exists for.
    /// `ui_sheet_gtk/src/code.rs` is the same file with a different address vocabulary, so this
    /// covers the shared half of both.
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

        // **The hang this split fixes.** `mark` is called from the handler that runs *because*
        // the cursor moved, so it must not move the cursor — a `place_cursor` there makes GTK
        // deliver the notify again and the window stops answering. Measured rather than
        // asserted in a comment: marking a different line leaves the cursor where it was.
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
        // The **first** paragraph is typed into the one a new document already has rather than
        // inserted in front of it: `grind_text::Document::default` is one empty paragraph, not
        // zero blocks, because a caret has to have somewhere to be.
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

    /// Links, all four halves: drawn underlined in the accent, a Ctrl+click on one hands its
    /// target over (and a click beside it does not), and Ctrl+K links a selection, edits the
    /// link at a bare caret, inserts a target as its own text when there is neither, and
    /// removes one.
    fn a_link_is_drawn_followed_and_edited() {
        use crate::metrics::run_attributes;
        use gtk::pango;

        let (doc, app) = shell(&["read the spec today"]);
        let imp = doc.imp();
        let at = |offset| Caret { block: 0, offset };
        app.set_link(at(9), at(13), Some("https://x.org")).unwrap();

        let view = app.get_viewport(0..1);
        let block = view.get(0).unwrap();
        let (layout, faces, _) = imp.measured(0).unwrap();
        let face = faces.of(&BlockKind::Paragraph, None);
        let drawn = |from: usize, to: usize| {
            let attrs = run_attributes(
                &block.runs,
                from,
                to,
                &block
                    .text
                    .chars()
                    .skip(from)
                    .take(to - from)
                    .collect::<String>(),
                face.size(),
                crate::metrics::Paper::LIGHT,
            );
            attrs
                .attributes()
                .iter()
                .map(|attr| attr.type_())
                .collect::<Vec<_>>()
        };
        let link = drawn(9, 13);
        assert!(link.contains(&pango::AttrType::Underline), "{link:?}");
        assert!(link.contains(&pango::AttrType::Foreground), "{link:?}");
        assert!(drawn(0, 9).is_empty(), "plain text is not");

        // A point on "spec" is on the link; one on "read" is not.
        let flow = imp.flow(600.0);
        let slot = flow.slot(0).unwrap();
        let (left, _) = crate::geom::column(600.0);
        let point = |offset: usize| {
            (
                left + slot.indent + f64::from(layout.x_at(offset)) + 2.0,
                slot.top + 2.0,
            )
        };
        let followed = Rc::new(RefCell::new(Vec::new()));
        doc.connect_link(glib::clone!(
            #[strong]
            followed,
            move |href| followed.borrow_mut().push(href)
        ));
        let (x, y) = point(10);
        assert!(imp.follow(x, y));
        let (x, y) = point(1);
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

    /// Enter at the bottom of the window keeps the caret on screen, as every word processor
    /// does. Reported from writing a README: the view stayed put and the caret went under the
    /// fold, because the edit's caret move followed a layout without the new line in it, and
    /// the scrollbar's range was still the shorter document's.
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
        let bottom = adjustment.value() + adjustment.page_size();
        let flow = imp.flow(600.0);
        let slot = flow.slot(60).expect("the last block is laid out");
        assert!(
            slot.top + slot.height <= bottom + 0.5,
            "the caret's line ({}..{}) is inside the view (..{bottom})",
            slot.top,
            slot.top + slot.height,
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
        let (layout, _, _) = imp.measured(0).expect("the block lays out");
        let flow = imp.flow(f64::from(doc.width()));
        let slot = flow.slot(0).expect("one block");
        let (left, _) = crate::geom::column(f64::from(doc.width()));
        let x_of = |offset: usize| left + f64::from(layout.x_at(offset));
        let y = slot.top + 1.0;

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
        let imp = doc.imp();
        let (layout, _, _) = imp.measured(0).expect("the block lays out");
        let flow = imp.flow(f64::from(doc.width()));
        let slot = flow.slot(0).expect("one block");
        let (left, _) = crate::geom::column(f64::from(doc.width()));
        (left + f64::from(layout.x_at(offset)), slot.top + 1.0)
    }

    /// The bug a screenshot found: a click plants the selection's anchor at the caret so a drag
    /// has somewhere to grow from, and typing used to leave it there — so the first character
    /// typed became a selection and the second replaced it. Clicking and typing `xy` wrote `y`,
    /// and two Backspaces after a click erased one character each side of it.
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

    /// A `Title`-styled paragraph gets its own, larger face — the same mechanism that makes
    /// a heading bigger than the body, keyed off the block's *name* instead of its kind
    /// because `Title` is `BlockKind::Paragraph` with nothing else to tell it apart.
    fn a_title_style_is_drawn_in_a_larger_face_than_the_body() {
        let (doc, app) = shell(&["Report", "body text"]);
        app.set_style(0..1, Some("Title".to_owned()))
            .expect("sets the style");
        let imp = doc.imp();
        let (_, faces, _) = imp.measured(0).expect("the title lays out");
        let (_, _, _) = imp.measured(1).expect("the body lays out");
        assert!(
            faces.of(&BlockKind::Paragraph, Some("Title")).height()
                > faces.of(&BlockKind::Paragraph, None).height(),
            "a title reads larger than a plain paragraph"
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

    /// A picture followed by its caption's text is still drawn as a picture — not the
    /// placeholder character a run of plain text next to an image draws everywhere else — with
    /// the flow reserving room under it for the caption, wrapped to the column. This is the
    /// real shape `doc/odt-format.md`'s "An inserted image is a frame inside a frame" reads,
    /// and the bug this pins is the caption vanishing rather than the picture.
    fn a_picture_with_a_caption_reserves_room_for_both() {
        let (doc, app) = shell(&[""]);
        app.insert_image(
            Caret {
                block: 0,
                offset: 0,
            },
            "image/png".to_owned(),
            DOT_PNG.to_vec(),
            None,
            None,
        )
        .expect("inserts");
        app.insert_text(
            Caret {
                block: 0,
                offset: 1,
            },
            "Figure 1: a photograph.",
        )
        .expect("inserts the caption");
        doc.invalidate();

        let imp = doc.imp();
        let width = f64::from(doc.width());
        let (_, column) = geom::column(width);
        let viewport = app.get_viewport(0..1);
        let block = viewport.get(0).expect("the block");
        let (image, caption) = picture_of(block).expect("still reads as a picture");
        assert_eq!(caption, Some("Figure 1: a photograph."));

        let texture = texture_of(image).expect("decodes");
        let (_, picture_height) = image_size(&texture, column);
        let text_height = caption_height(imp.faces().body(), caption.unwrap(), column);

        let flow = imp.flow(width);
        let slot = flow.slot(0).expect("one block");
        assert!(
            slot.height >= picture_height + text_height,
            "the flow leaves room for the picture and its caption, not just one of them"
        );
    }

    /// A block that is only an image is drawn as one — decoded, scaled to fit the column, and
    /// tall enough in the flow that nothing after it overlaps.
    fn a_block_that_is_only_an_image_is_sized_from_the_picture_not_a_line_of_text() {
        let (doc, app) = shell(&[""]);
        app.insert_image(
            Caret {
                block: 0,
                offset: 0,
            },
            "image/png".to_owned(),
            DOT_PNG.to_vec(),
            None,
            None,
        )
        .expect("inserts");
        // No observer is wired up in this harness, so nothing calls this on its own the way a
        // real window's bridge would — `Doc::flow`'s cache otherwise still holds the empty
        // paragraph's height from `shell`'s own `allocate`.
        doc.invalidate();

        let imp = doc.imp();
        let width = f64::from(doc.width());
        let (_, column) = geom::column(width);
        let viewport = app.get_viewport(0..1);
        let block = viewport.get(0).expect("the block");
        assert!(picture_of(block).is_some(), "the whole block is the image");

        let texture = texture_of(picture_of(block).unwrap().0).expect("decodes");
        let (w, h) = image_size(&texture, column);
        assert!((w - h).abs() < 0.01, "a 4x4 image stays square when scaled");
        assert!(
            w <= 4.0,
            "a 4-pixel-wide image is never stretched past its own size"
        );

        // The flow's own height for this block has to come from the same arithmetic, not from
        // laying out the empty paragraph the image replaced — which would give one line of the
        // body face (`imp.faces().body().height()`), a different number for almost any image.
        let flow = imp.flow(width);
        let slot = flow.slot(0).expect("one block");
        assert!(
            (slot.height - h).abs() < 0.01,
            "the flow's height for this block is the picture's ({h}), not {}",
            imp.faces().body().height()
        );
    }

    /// The test S9 exists for, and the GTK half of `doc/text-layout.md`'s payoff: Down is
    /// not "the next block", it is the next *line*, and the answer comes from the core —
    /// measured in pixels through Pango here and in cells in the terminal.
    fn down_moves_by_a_wrapped_line_not_by_a_block() {
        let long = "the cat sat on the mat and then it slept for a very long time indeed \
                    while the rain kept on falling outside the window all afternoon";
        let (doc, app) = shell(&[long, "after"]);
        let imp = doc.imp();
        let (layout, faces, kind) = imp.measured(0).expect("the block lays out");
        assert!(
            layout.lines().len() > 1,
            "the fixture has to actually wrap or this proves nothing"
        );
        drop((faces, kind));

        imp.go(crate::keymap::Motion::Line(1), false);
        assert_eq!(doc.caret().block, 0, "still inside the same paragraph");
        assert!(doc.caret().offset > 0, "but further down it");

        // Off the end of the last line, the same key carries into the next block — a
        // document is one flow, not a list of boxes.
        for _ in 0..layout.lines().len() {
            imp.go(crate::keymap::Motion::Line(1), false);
        }
        assert_eq!(doc.caret().block, 1);
        assert_eq!(app.block_count(), 2, "and nothing was edited on the way");
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

    /// Both halves of the backtick notation, in the *window* — which is the half that was
    /// reported broken (`text/src/markdown.rs`). The file was already right, so nothing here
    /// asserts about the document: what it asserts is that this shell **measures** a monospace
    /// run in a monospace face and puts the same family on the attributes it **draws** with.
    /// Either alone would be a bug — a family drawn but not measured drifts the caret through
    /// the run, and one measured but not drawn is invisible.
    fn code_is_measured_and_drawn_in_a_monospace_face() {
        use crate::metrics::run_attributes;
        use grind_core::style::TextStyle;
        use grind_text::Metrics;
        use gtk::pango;

        let (doc, app) = shell(&[""]);
        let imp = doc.imp();
        imp.move_caret(
            Caret {
                block: 0,
                offset: 0,
            },
            true,
        );
        for c in "run `ls -l` now".chars() {
            imp.type_text(&c.to_string());
        }
        let view = app.get_viewport(0..1);
        let block = view.get(0).expect("the block");

        // Measured. Proportional and monospace disagree about this string by construction:
        // `i` is the narrowest letter there is and `w` the widest, and a monospace face is
        // exactly the one that makes them equal.
        let (_, faces, _) = imp.measured(0).expect("the paragraph lays out");
        let face = faces.of(&BlockKind::Paragraph, None);
        let mono = TextStyle {
            font_family: Some(grind_text::markdown::MONOSPACE.to_owned()),
            ..TextStyle::default()
        };
        let widths = |style: &TextStyle| {
            let mut out = Vec::new();
            face.advances("iiiiwwww", style, &mut out);
            *out.last().expect("one advance per character")
        };
        assert_ne!(
            widths(&TextStyle::default()),
            widths(&mono),
            "a fragment is measured in its own family, not in the block's"
        );

        // Drawn. The family reaches the Pango attribute list the line is painted with.
        let attrs = run_attributes(
            &block.runs,
            0,
            block.text.chars().count(),
            &block.text,
            face.size(),
            crate::metrics::Paper::LIGHT,
        );
        assert!(
            attrs
                .attributes()
                .iter()
                .any(|attr| attr.type_() == pango::AttrType::Family),
            "the `code` run carries a family attribute"
        );

        // And the block half: a fence is a paragraph style, and it has a face of its own.
        let fenced = faces.of(
            &BlockKind::Paragraph,
            Some(grind_text::markdown::PREFORMATTED),
        );
        let mut plain_width = Vec::new();
        let mut fenced_width = Vec::new();
        face.advances("iiiiwwww", &TextStyle::default(), &mut plain_width);
        fenced.advances("iiiiwwww", &TextStyle::default(), &mut fenced_width);
        assert_ne!(
            plain_width.last(),
            fenced_width.last(),
            "a fenced block is set in its own face rather than the body's"
        );
    }

    /// The gap `doc/text-shell.md` used to name for this shell alone: a document that coloured
    /// a word drew it in the theme's own ink, and a size was neither measured nor drawn. All
    /// three reach the attribute list now, and the size reaches the *height* with them — the
    /// half that made honouring it a real change rather than one attribute.
    fn a_runs_colour_highlight_and_size_are_drawn() {
        use crate::metrics::run_attributes;
        use grind_core::style::TextStyle;
        use grind_text::Metrics;
        use gtk::pango;

        let (doc, app) = shell(&["coloured and large"]);
        let imp = doc.imp();
        let span = |from: usize, to: usize| {
            (
                Caret {
                    block: 0,
                    offset: from,
                },
                Caret {
                    block: 0,
                    offset: to,
                },
            )
        };
        let (from, to) = span(0, 8);
        app.set_char_style(
            from,
            to,
            &grind_text::CharStyle {
                color: Some("#ff4136".into()),
                background: Some("#ffdc00".into()),
                font_size: Some("24pt".into()),
                ..Default::default()
            },
        )
        .expect("formats");

        let view = app.get_viewport(0..1);
        let block = view.get(0).expect("the block");
        let (_, faces, _) = imp.measured(0).expect("the paragraph lays out");
        let face = faces.of(&BlockKind::Paragraph, None);
        let attrs = run_attributes(
            &block.runs,
            0,
            block.text.chars().count(),
            &block.text,
            face.size(),
            crate::metrics::Paper::LIGHT,
        );
        let kinds: Vec<pango::AttrType> =
            attrs.attributes().iter().map(|attr| attr.type_()).collect();
        for wanted in [
            pango::AttrType::Foreground,
            pango::AttrType::Background,
            pango::AttrType::Size,
        ] {
            assert!(kinds.contains(&wanted), "{wanted:?} is drawn: {kinds:?}");
        }

        // And measured: a bigger size is a taller line, or the run would overprint the one
        // above it — which is why this file did not honour a size at all until it honoured
        // all three halves of one.
        let big = TextStyle {
            font_size: Some("24pt".into()),
            ..TextStyle::default()
        };
        assert!(
            face.line_height(&big) > face.line_height(&TextStyle::default()),
            "a 24pt run makes room for itself"
        );
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

    /// The gap this shell used to be alone in having: neither a system clipboard nor a
    /// register. What is checked here is the two halves either side of `gdk::Clipboard` — a
    /// selection as plain text, and plain text back into blocks — because the clipboard itself
    /// is asynchronous and belongs to a display server rather than to this program.
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
