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

    /// The document changed: forget everything measured from it and repaint.
    ///
    /// Called from the observer, so it covers this widget's own edits too — they reach the
    /// core the same way anything else does (doc/plan.md rule 3).
    pub fn invalidate(&self) {
        self.imp().flow.replace(None);
        self.imp().clamp_caret();
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

    /// Told whenever the caret moves — the status bar's readout.
    pub fn connect_moved(&self, f: impl Fn(Caret) + 'static) {
        self.imp().on_moved.borrow_mut().push(Box::new(f));
    }

    /// Told when an edit was refused, with the core's own message. A toast, never a dialog:
    /// nothing here is a question.
    pub fn connect_notice(&self, f: impl Fn(String) + 'static) {
        self.imp().on_notice.borrow_mut().push(Box::new(f));
    }

    /// Set the caret from an address the user typed — `p12`, `#intro`, `§2.1.3`. A jump
    /// replaces the caret rather than extending anything, so any selection goes with it.
    pub fn go_to(&self, caret: Caret) {
        self.imp().anchor.set(None);
        self.imp().move_caret(caret, true);
    }
}

mod imp {
    use super::*;

    use grind_core::layout::{Layout, Line};
    use grind_text::{BlockKind, loc};
    use gtk::graphene;
    use gtk::pango;
    use gtk::subclass::prelude::*;

    use crate::geom;
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
        pub flow: RefCell<Option<(f64, Rc<Flow>)>>,
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
        pub on_moved: RefCell<Vec<MovedHook>>,
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
                on_moved: RefCell::new(Vec::new()),
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
                    doc.imp().click(x, y, shift);
                }
            ));
            drag.connect_drag_update(glib::clone!(
                #[weak(rename_to = doc)]
                widget,
                move |gesture, offset_x, offset_y| {
                    if let Some((start_x, start_y)) = gesture.start_point() {
                        doc.imp().drag_to(start_x + offset_x, start_y + offset_y);
                    }
                }
            ));
            widget.add_controller(drag);
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
            let flow = self.flow(f64::from(width));
            configure(
                self.vadjustment.borrow().as_ref(),
                f64::from(height).max(1.0),
                flow.height() + geom::MARGIN,
                self.faces().body().height(),
            );
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
            let (left, column) = geom::column(width);
            let scroll = self.scroll();

            snapshot.append_color(&palette.background, &rect(0.0, 0.0, width, height));

            let slots = flow.visible(scroll, scroll + height);
            let Some((first, last)) = slots.first().zip(slots.last()) else {
                return;
            };
            // One read for the whole paint, exactly as wide as the screen (rule 1).
            let viewport = app.get_viewport(first.index..last.index + 1);
            let caret = self.caret.get();
            let selection = self.selection();

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
                    let (w, h) = image_size(&texture, column - slot.indent);
                    snapshot.append_texture(&texture, &rect(x, y, w, h));
                    if selection.is_none() && slot.index == caret.block && widget.is_focus() {
                        snapshot.append_color(&palette.accent, &rect(x, y, CARET, h));
                    }
                    if let Some(caption) = caption {
                        let caption_y = y + h + CAPTION_GAP;
                        draw_at(
                            snapshot,
                            faces.body().draw_wrapped(caption, column - slot.indent),
                            x,
                            caption_y,
                            palette.dim,
                        );
                    }
                    continue;
                }

                let style = block.style.as_deref();
                let face = faces.of(&block.kind, style);
                let Ok(layout) = app.layout_block(slot.index, (column - slot.indent) as f32, face)
                else {
                    continue;
                };
                let text: Vec<char> = block.text.chars().collect();
                let ink = match style {
                    Some("Subtitle") => palette.dim,
                    _ => palette.foreground,
                };

                if let BlockKind::ListItem { .. } = block.kind {
                    // A bullet is drawn rather than inserted: the character is not in the
                    // document, and putting one there would make it a character the caret
                    // could sit inside and a `p12+0` that means something else.
                    draw_at(
                        snapshot,
                        face.draw("\u{2022}"),
                        x - BULLET_GAP,
                        y,
                        palette.dim,
                    );
                }

                // The selection's band, one rectangle per line it crosses, drawn under the
                // text so a run painted over it stays legible.
                if let Some((from, to)) = selection
                    && slot.index >= from.block
                    && slot.index <= to.block
                {
                    let start = if slot.index == from.block {
                        from.offset
                    } else {
                        0
                    };
                    let end = if slot.index == to.block {
                        to.offset
                    } else {
                        text.len()
                    };
                    for line in layout.lines() {
                        if let Some((left, width)) = selection_span(&layout, line, start, end) {
                            snapshot.append_color(
                                &palette.selection,
                                &rect(
                                    x + f64::from(left),
                                    y + f64::from(line.top),
                                    f64::from(width),
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
                    let attrs = run_attributes(&block.runs, line.start, line.end, piece);
                    draw_at(
                        snapshot,
                        face.draw_styled(piece, &attrs),
                        x,
                        y + f64::from(line.top),
                        ink,
                    );
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
            let (_, column) = geom::column(width);
            if let Some((measured, flow)) = self.flow.borrow().as_ref()
                && *measured == column
            {
                return flow.clone();
            }
            let flow = Rc::new(self.build_flow(column));
            self.flow.replace(Some((column, flow.clone())));
            flow
        }

        fn build_flow(&self, column: f64) -> Flow {
            // The page's top margin is part of the flow rather than a fixed band, so it
            // scrolls away with the text the way the top of a page does.
            let mut flow = Flow::new(geom::MARGIN);
            let Some(app) = self.app() else { return flow };
            let faces = self.faces();
            let viewport = app.get_viewport(0..app.block_count());
            for block in viewport.iter() {
                let style = block.style.as_deref();
                let indent = indent_of(&block.kind);
                let height = match picture_of(block)
                    .and_then(|(image, caption)| Some((texture_of(image)?, caption)))
                {
                    Some((texture, caption)) => {
                        let picture = image_size(&texture, column - indent).1;
                        match caption {
                            Some(caption) => {
                                picture
                                    + CAPTION_GAP
                                    + caption_height(faces.body(), caption, column - indent)
                            }
                            None => picture,
                        }
                    }
                    None => {
                        let face = faces.of(&block.kind, style);
                        app.layout_block(block.index, (column - indent) as f32, face)
                            .map(|layout| f64::from(layout.height()))
                            .unwrap_or_else(|_| face.height())
                    }
                };
                let space = match (style, &block.kind) {
                    (Some("Title" | "Subtitle"), _) | (_, BlockKind::Heading { .. }) => {
                        geom::HEADING_GAP
                    }
                    _ => geom::GAP,
                };
                flow.push(block.index, height, indent, space, geom::GAP);
            }
            flow
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
            let (_, column) = geom::column(f64::from(self.obj().width()));
            let width = (column - indent_of(&kind)) as f32;
            let layout = app
                .layout_block(index, width, faces.of(&kind, style.as_deref()))
                .ok()?;
            Some((layout, faces, kind))
        }

        /// How each block is set — this window's [`grind_text::Faces`], which is what every
        /// motion by line is asked through.
        ///
        /// Rebuilt per question rather than kept, because both halves of it change under the
        /// window: the faces on a theme change, the column on a resize.
        fn column(&self) -> Column {
            let (_, column) = geom::column(f64::from(self.obj().width()));
            Column {
                faces: self.faces(),
                column,
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
            }
            glib::Propagation::Stop
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
                Motion::DocStart => self.move_caret(
                    Caret {
                        block: 0,
                        offset: 0,
                    },
                    true,
                ),
                Motion::DocEnd => {
                    let block = app.block_count() - 1;
                    self.move_caret(
                        Caret {
                            block,
                            offset: self.block_len(&app, block),
                        },
                        true,
                    );
                }
            }
        }

        /// One character left or right, rolling onto the neighbouring block at either end.
        ///
        /// The only arithmetic in this file, and it is over *characters* rather than over
        /// layout — walking off the end of a block is a document fact, not a line one.
        fn stepped(&self, app: &App, delta: i32) -> Caret {
            let mut caret = self.caret.get();
            if delta > 0 {
                if caret.offset < self.block_len(app, caret.block) {
                    caret.offset += 1;
                } else if caret.block + 1 < app.block_count() {
                    caret = Caret {
                        block: caret.block + 1,
                        offset: 0,
                    };
                }
            } else if caret.offset > 0 {
                caret.offset -= 1;
            } else if caret.block > 0 {
                caret = Caret {
                    block: caret.block - 1,
                    offset: self.block_len(app, caret.block - 1),
                };
            }
            caret
        }

        fn block_len(&self, app: &App, index: usize) -> usize {
            app.input_text(index)
                .map(|text| text.chars().count())
                .unwrap_or(0)
        }

        /// The block and offset a point in the widget lands on: which block the pointer is
        /// over, then which line, then where on it — the same two questions in the same
        /// order the core answers them, so a click and a Down-arrow land on the same offset.
        fn caret_at(&self, x: f64, y: f64) -> Option<Caret> {
            let width = f64::from(self.obj().width());
            let flow = self.flow(width);
            let (left, _) = geom::column(width);
            let scroll = self.scroll();
            let slot = flow.at_y(y + scroll).and_then(|index| flow.slot(index))?;
            let (layout, _, _) = self.measured(slot.index)?;
            let line = line_at_y(&layout, y + scroll - slot.top);
            let offset = layout.offset_at(line, (x - left - slot.indent) as f32);
            Some(Caret {
                block: slot.index,
                offset,
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

        // --- editing ---

        /// If there is a selection, erase it and hand back the caret it collapses to —
        /// what typing a character, or Enter, over a selection does in every editor. Returns
        /// the caret unchanged when there is nothing selected.
        fn consume_selection(&self, app: &App) -> Caret {
            let Some((from, to)) = self.selection() else {
                return self.caret.get();
            };
            self.anchor.set(None);
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
                    self.move_caret(typed.caret, true);
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
        pub fn move_caret(&self, caret: Caret, clear_goal: bool) {
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

        fn scroll_into_view(&self) {
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
            let Some(adjustment) = self.vadjustment.borrow().clone() else {
                return;
            };
            let page = f64::from(widget.height());
            adjustment.set_value(flow.follow(adjustment.value(), page, target));
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
            self.obj()
                .announce(&message, gtk::AccessibleAnnouncementPriority::Medium);
        }

        fn notice(&self, message: String) {
            for hook in self.on_notice.borrow().iter() {
                hook(message.clone());
            }
        }
    }

    /// The highlighted band's left edge and width for one line of a selection spanning
    /// `start..end` (both block-relative character offsets), or `None` where the line has
    /// nothing selected on it.
    ///
    /// `Layout::x_at` resolves an offset sitting exactly at a soft break to **the next
    /// line's** start (`grind_core::layout::Line::line_at`'s own doc comment) — the right
    /// convention for a caret walking off a wrapped line, and the wrong one for *this* line's
    /// own right edge: asking it for `line.end` on every line but the last would silently
    /// hand back 0, collapsing every one of them to a zero-width band. `Line::width` is that
    /// same distance with no such ambiguity, so it stands in whenever the selection reaches
    /// all the way to this line's own end.
    fn selection_span(
        layout: &Layout,
        line: &Line,
        start: usize,
        end: usize,
    ) -> Option<(f32, f32)> {
        let from_x = start.max(line.start);
        let to_x = end.min(line.end);
        if from_x >= to_x {
            return None;
        }
        let left = layout.x_at(from_x);
        let right = match to_x == line.end {
            true => line.width,
            false => layout.x_at(to_x),
        };
        Some((left, right - left))
    }

    /// Whether a block is a picture, optionally followed by its caption's plain text — the
    /// shape `App::insert_image` produces into an empty paragraph (no caption) and the shape
    /// a real ODF frame reads as (an image run, then the caption paragraph's text, `doc/
    /// odt-format.md`'s "An inserted image is a frame inside a frame"). Both are drawn as a
    /// picture rather than the placeholder character every other block context sees. An image
    /// sitting mid-sentence with other text around it still draws as `\u{fffc}`, which is the
    /// gap `doc/text-shell.md` names.
    pub(super) fn picture_of(
        block: &grind_text::BlockView,
    ) -> Option<(&grind_text::ImageView, Option<&str>)> {
        match block.runs.as_slice() {
            [run] => run.image.as_ref().map(|image| (image, None)),
            [run, caption] if caption.image.is_none() => run
                .image
                .as_ref()
                .map(|image| (image, Some(caption.text.as_str()))),
            _ => None,
        }
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
    }

    impl grind_text::Faces for Column {
        fn of(
            &self,
            _index: usize,
            kind: &BlockKind,
            style: Option<&str>,
        ) -> (f32, &dyn grind_text::Metrics) {
            (
                (self.column - indent_of(kind)) as f32,
                self.faces.of(kind, style),
            )
        }
    }

    /// How far a block's text is indented — a list's nesting, and nothing else.
    fn indent_of(kind: &BlockKind) -> f64 {
        match kind {
            BlockKind::ListItem { depth } => f64::from(*depth) * geom::INDENT,
            _ => 0.0,
        }
    }

    /// Which line of a layout a y coordinate is on, measured from the block's own top.
    fn line_at_y(layout: &Layout, y: f64) -> usize {
        let last = layout.lines().len().saturating_sub(1);
        layout
            .lines()
            .iter()
            .position(|line| y < f64::from(line.top + line.height))
            .unwrap_or(last)
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
            K::BackSpace => Key::Backspace,
            K::Delete | K::KP_Delete => Key::Delete,
            _ => Key::Other,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use grind_core::layout::{Fixed, Fragment, wrap};
        use grind_core::style::TextStyle;

        /// `Layout` is the core's, so this checks only the shell's own reading of it: which
        /// line a y coordinate falls on, including above and below the block.
        #[test]
        fn a_y_coordinate_picks_the_line_it_is_inside() {
            let style = TextStyle::default();
            let layout = wrap(
                &[Fragment {
                    text: "the cat sat on the mat",
                    style: &style,
                }],
                10.0,
                &Fixed,
            );
            assert!(layout.lines().len() > 1, "the fixture has to wrap");
            assert_eq!(line_at_y(&layout, 0.0), 0);
            assert_eq!(line_at_y(&layout, 1.5), 1, "one unit per line under Fixed");
            assert_eq!(line_at_y(&layout, -50.0), 0, "above the block");
            assert_eq!(
                line_at_y(&layout, 500.0),
                layout.lines().len() - 1,
                "below it"
            );
        }

        /// The bug a screenshot found: a selection spanning a soft line break (a `text:tab`
        /// and `text:line-break` inside one paragraph both force one — `p12` typed as
        /// `printf 'name\tvalue\nsecond line'` is exactly this) drew every line but the
        /// selection's *last* as a zero-width band, because `Layout::x_at(line.end)`
        /// resolves that boundary offset to the **next** line rather than this one's own
        /// right edge.
        #[test]
        fn a_selection_ending_at_a_soft_break_still_highlights_that_lines_own_width() {
            let style = TextStyle::default();
            let layout = wrap(
                &[Fragment {
                    text: "name\tvalue\nsecond line",
                    style: &style,
                }],
                1000.0,
                &Fixed,
            );
            assert_eq!(
                layout.lines().len(),
                2,
                "the line break forces a second visual line"
            );
            let first = &layout.lines()[0];
            let (_, width) = selection_span(&layout, first, 0, layout.len())
                .expect("the whole block, including its first line, is selected");
            assert!(
                width > 0.0,
                "the first line's own highlight must not collapse to zero"
            );
        }

        #[test]
        fn only_a_list_item_is_indented_and_it_is_by_its_depth() {
            assert_eq!(indent_of(&BlockKind::Paragraph), 0.0);
            assert_eq!(indent_of(&BlockKind::Heading { level: 1 }), 0.0);
            assert_eq!(
                indent_of(&BlockKind::ListItem { depth: 2 }),
                2.0 * geom::INDENT
            );
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
            "the code view shows the projection, tagged and marked",
            the_code_view_shows_the_projection,
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
        for (index, text) in paragraphs.iter().enumerate() {
            app.insert(index, BlockKind::Paragraph, text)
                .expect("inserts");
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
